"""
feed_search.py — BM25 search engine with evidence scoring.

Provides Tavily-compatible search responses for AI agents,
with relevance scoring, evidence snippets, and follow-up suggestions.
"""

from __future__ import annotations

import json
import logging
import math
import os
import re
import time
from collections import Counter
from datetime import datetime, timezone
from typing import Any

from feed_security import detect_injection, normalize_unicode, sanitize_for_llm
from feed_utils import get_db

logger = logging.getLogger("feed_search")


def _tokenize(text: str) -> list[str]:
    text = normalize_unicode(text.lower())
    tokens = re.findall(r'[a-z0-9]+', text)
    return [t for t in tokens if len(t) > 1]


def _bm25_score(query_tokens: list[str], doc_tokens: list[str],
                avg_dl: float, k1: float = 1.5, b: float = 0.75) -> float:
    doc_len = len(doc_tokens)
    doc_tf = Counter(doc_tokens)
    score = 0.0

    for qt in query_tokens:
        if qt not in doc_tf:
            continue
        tf = doc_tf[qt]
        numerator = tf * (k1 + 1)
        denominator = tf + k1 * (1 - b + b * doc_len / max(avg_dl, 1))
        score += numerator / denominator

    return score


def _compute_recency(published_at: str | None) -> float:
    if not published_at:
        return 0.3
    try:
        if isinstance(published_at, str):
            dt = datetime.fromisoformat(published_at.replace("Z", "+00:00"))
        else:
            dt = published_at
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        hours_old = (datetime.now(timezone.utc) - dt).total_seconds() / 3600
        if hours_old < 1:
            return 1.0
        if hours_old < 24:
            return 0.8
        if hours_old < 168:
            return 0.6
        if hours_old < 720:
            return 0.4
        return 0.2
    except (ValueError, TypeError):
        return 0.3


def _extract_highlights(content: str, query_tokens: list[str], max_highlights: int = 3) -> list[str]:
    if not content:
        return []
    sentences = re.split(r'[.!?]+', content)
    scored = []
    for sent in sentences:
        sent = sent.strip()
        if not sent or len(sent) < 20:
            continue
        sent_tokens = set(_tokenize(sent))
        overlap = len(set(query_tokens) & sent_tokens)
        if overlap > 0:
            scored.append((overlap, sent))

    scored.sort(key=lambda x: x[0], reverse=True)
    return [s[1][:300] for s in scored[:max_highlights]]


def _extract_evidence_excerpts(content: str, query_tokens: list[str], max_excerpts: int = 3) -> list[dict]:
    if not content:
        return []
    excerpts = []
    paragraphs = content.split("\n\n")
    if len(paragraphs) == 1:
        paragraphs = content.split("\n")

    for para in paragraphs:
        para = para.strip()
        if not para or len(para) < 30:
            continue
        para_tokens = set(_tokenize(para))
        overlap = len(set(query_tokens) & para_tokens)
        if overlap > 0:
            relevance = min(overlap / max(len(query_tokens), 1), 1.0)
            excerpts.append({"text": para[:500], "relevance": round(relevance, 3)})

    excerpts.sort(key=lambda x: x["relevance"], reverse=True)
    return excerpts[:max_excerpts]


def _count_corroboration(key_phrases: list[str], query_tokens: list[str]) -> int:
    if not key_phrases:
        return 0
    phrase_tokens = set()
    for kp in key_phrases:
        phrase_tokens.update(_tokenize(kp))
    return len(set(query_tokens) & phrase_tokens)


def search(
    query: str,
    tags: list[str] | None = None,
    since: str | None = None,
    limit: int = 10,
    source: str = "",
    include_answer: str | bool = False,
    include_raw_content: bool = False,
) -> dict:
    """
    Tavily-compatible search across feed items.

    Args:
        query: Search query
        tags: Filter by tags
        since: ISO date string — only items after this date
        limit: Max results to return
        source: Filter by source name
        include_answer: False, "extractive", or "synthetic"
        include_raw_content: Include full item content
    """
    start_time = time.time()

    detected, patterns = detect_injection(query)
    if detected:
        logger.warning("Injection detected in search query, sanitizing")
        query = sanitize_for_llm(query, max_length=200)

    query_tokens = _tokenize(query)
    if not query_tokens:
        return _empty_response(query)

    con = get_db(read_only=True)

    workspace = os.environ.get("VAK_FEED_WORKSPACE", "")
    conditions = [
        "f.removed_at IS NULL",
        "i.security_status = 'accepted'",
        "(f.scope = 'global' OR f.workspace_id = ?)",
    ]
    params: list[Any] = [workspace]

    # Select by lexical evidence before ranking.  The previous implementation
    # ranked only the newest 500 rows, which made an older exact match vanish
    # behind unrelated fresh items.  BM25 remains the ranking function, but it
    # must operate on the complete matching candidate set.
    token_clauses = [
        "lower(coalesce(i.title, '') || ' ' || coalesce(i.summary, '') || ' ' || coalesce(i.content, '')) LIKE ?"
        for _ in query_tokens
    ]
    conditions.append("(" + " OR ".join(token_clauses) + ")")
    params.extend(f"%{token}%" for token in query_tokens)

    if tags:
        conditions.append("i.tags && ?")
        params.append(tags)

    if since:
        conditions.append("i.published_at >= ?")
        params.append(since)

    if source:
        conditions.append("f.name = ?")
        params.append(source)

    where = " AND ".join(conditions)

    count_query = f"""
        SELECT COUNT(DISTINCT i.id)
        FROM items i
        LEFT JOIN feeds f ON i.feed_id = f.id
        WHERE {where}
    """
    total_results = con.execute(count_query, params).fetchone()[0]

    search_query = f"""
        SELECT i.id, i.title, i.url, i.author, i.summary, i.content,
               i.published_at, i.tags, i.source_trust, i.word_count,
               i.key_phrases, f.name as source_name, f.source_type
        FROM items i
        LEFT JOIN feeds f ON i.feed_id = f.id
        WHERE {where}
        ORDER BY i.published_at DESC NULLS LAST
    """
    rows = con.execute(search_query, params).fetchall()
    con.close()

    avg_dl = sum(r[9] or 0 for r in rows) / max(len(rows), 1)

    scored_results = []
    seen_urls: set[str] = set()
    dedup_count = 0
    for row in rows:
        item_id = row[0]
        title = row[1] or ""
        url = row[2] or ""
        author = row[3] or ""
        summary = row[4] or ""
        content = row[5] or ""
        published_at = str(row[6]) if row[6] else None
        tags_list = row[7] or []
        trust = row[8] or "medium"
        word_count = row[9] or 0
        key_phrases = row[10] or []
        source_name = row[11] or ""
        source_type = row[12] or ""

        if url in seen_urls:
            dedup_count += 1
            continue
        seen_urls.add(url)

        searchable = f"{title} {summary} {content}"
        doc_tokens = _tokenize(searchable)

        bm25 = _bm25_score(query_tokens, doc_tokens, avg_dl)
        recency = _compute_recency(published_at)
        trust_map = {"high": 1.0, "medium": 0.6, "low": 0.3}
        trust_score = trust_map.get(trust, 0.5)
        engagement = min((word_count or 0) / 1000, 1.0)

        composite_score = (bm25 * 0.6) + (recency * 0.2) + (trust_score * 0.1) + (engagement * 0.1)

        # Freshness and trust rank matches; they cannot create a match.
        if bm25 <= 0:
            continue

        highlights = _extract_highlights(content, query_tokens)
        evidence_excerpts = _extract_evidence_excerpts(content, query_tokens)
        corroboration = _count_corroboration(key_phrases, query_tokens)

        result = {
            "url": url,
            "title": title,
            "content": summary[:500] if summary else content[:500],
            "score": round(composite_score, 4),
            "published_date": published_at[:10] if published_at else None,
            "source_name": source_name,
            "source_type": source_type,
            "tags": tags_list,
            "highlights": highlights,
            "evidence": {
                "excerpts": evidence_excerpts,
                "source_trust": trust,
                "freshness_hours": _compute_freshness_hours(published_at),
                "corroboration_count": corroboration,
            },
        }

        if include_raw_content:
            result["raw_content"] = content[:50000]

        scored_results.append(result)

    scored_results.sort(key=lambda x: x["score"], reverse=True)
    top_results = scored_results[:limit]

    answer = None
    if include_answer and top_results:
        answer = _generate_extractive_answer(query, top_results)

    follow_ups = _generate_follow_ups(query, top_results)

    elapsed_ms = int((time.time() - start_time) * 1000)

    response = {
        "query": query,
        "answer": answer,
        "follow_up_questions": follow_ups,
        "results": top_results,
        "meta": {
            "total_results": total_results,
            "returned_results": len(top_results),
            "search_time_ms": elapsed_ms,
            "sources_searched": list(set(r["source_name"] for r in top_results if r["source_name"])),
            "deduplicated": dedup_count,
        },
    }

    return response


def _empty_response(query: str) -> dict:
    return {
        "query": query,
        "answer": None,
        "follow_up_questions": [],
        "results": [],
        "meta": {
            "total_results": 0,
            "returned_results": 0,
            "search_time_ms": 0,
            "sources_searched": [],
            "deduplicated": 0,
        },
    }


def _compute_freshness_hours(published_at: str | None) -> int:
    if not published_at:
        return -1
    try:
        dt = datetime.fromisoformat(published_at.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return int((datetime.now(timezone.utc) - dt).total_seconds() / 3600)
    except (ValueError, TypeError):
        return -1


def _generate_extractive_answer(query: str, results: list[dict]) -> str:
    if not results:
        return ""
    best = results[0]
    content = best.get("content", "")
    if len(content) > 500:
        content = content[:500].rsplit(" ", 1)[0] + "..."
    return content


def _generate_follow_ups(query: str, results: list[dict]) -> list[str]:
    follow_ups = []
    if not results:
        return follow_ups

    all_tags = set()
    for r in results:
        all_tags.update(r.get("tags", []))

    query_lower = query.lower()
    for tag in list(all_tags)[:5]:
        suggestion = f"What are the latest developments in {tag}?"
        if tag.lower() not in query_lower:
            follow_ups.append(suggestion)

    return follow_ups[:3]


def get_latest(
    source: str = "",
    tags: list[str] | None = None,
    limit: int = 20,
) -> dict:
    con = get_db(read_only=True)
    conditions = ["1=1"]
    params: list[Any] = []

    if source:
        conditions.append("f.name = ?")
        params.append(source)
    if tags:
        conditions.append("i.tags && ?")
        params.append(tags)

    where = " AND ".join(conditions)
    query = f"""
        SELECT i.id, i.title, i.url, i.author, i.summary, i.published_at,
               i.tags, i.source_trust, i.word_count,
               f.name as source_name, f.source_type
        FROM items i
        LEFT JOIN feeds f ON i.feed_id = f.id
        WHERE {where}
        ORDER BY i.published_at DESC NULLS LAST
        LIMIT ?
    """
    params.append(limit)
    rows = con.execute(query, params).fetchall()
    con.close()

    results = []
    for row in rows:
        results.append({
            "url": row[2] or "",
            "title": row[1] or "",
            "content": (row[4] or "")[:500],
            "score": 0.0,
            "published_date": str(row[5])[:10] if row[5] else None,
            "source_name": row[9] or "",
            "source_type": row[10] or "",
            "tags": row[6] or [],
            "highlights": [],
            "evidence": {
                "excerpts": [],
                "source_trust": row[7] or "medium",
                "freshness_hours": _compute_freshness_hours(str(row[5]) if row[5] else None),
                "corroboration_count": 0,
            },
        })

    return {
        "query": "",
        "answer": None,
        "follow_up_questions": [],
        "results": results,
        "meta": {
            "total_results": len(results),
            "returned_results": len(results),
            "search_time_ms": 0,
            "sources_searched": [source] if source else [],
            "deduplicated": 0,
        },
    }
