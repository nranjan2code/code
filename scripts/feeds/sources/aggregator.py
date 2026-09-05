"""
Aggregator driver for Hacker News, Reddit, and Lobsters.
Uses public JSON APIs — no API keys required for read access.
"""

from __future__ import annotations

import logging
import time
from datetime import datetime, timezone
from typing import Any

import requests

from feed_security import detect_injection, log_security_event, normalize_unicode, sanitize_html, validate_url

logger = logging.getLogger("feed_sources.aggregator")

FETCH_TIMEOUT = 15
MAX_ITEMS = 50


class AggregatorDriver:
    source_type = "aggregator"

    def fetch(self, driver: str, variant: str = "top", tags: list[str] | None = None,
              trust: str = "medium", max_items: int = MAX_ITEMS, **kwargs) -> list[dict]:
        drivers = {
            "hacker_news": self._fetch_hn,
            "reddit": self._fetch_reddit,
            "lobsters": self._fetch_lobsters,
        }
        fn = drivers.get(driver)
        if not fn:
            logger.error("Unknown aggregator driver: %s", driver)
            return []
        return fn(variant, tags or [], trust, max_items, **kwargs)

    def _fetch_hn(self, variant: str, tags: list[str], trust: str, max_items: int, **kwargs) -> list[dict]:
        endpoints = {
            "top": "topstories",
            "new": "newstories",
            "best": "beststories",
            "ask": "askstories",
            "show": "showstories",
        }
        endpoint = endpoints.get(variant, "topstories")
        url = f"https://hacker-news.firebaseio.com/v0/{endpoint}.json"

        ok, reason = validate_url(url)
        if not ok:
            log_security_event("ssrf_blocked", url=url, reason=reason)
            return []

        try:
            resp = requests.get(url, timeout=FETCH_TIMEOUT, allow_redirects=False)
            resp.raise_for_status()
            story_ids = resp.json()[:max_items]
        except (requests.RequestException, ValueError) as e:
            logger.error("Failed to fetch HN %s: %s", endpoint, e)
            return []

        items = []
        for story_id in story_ids:
            item_url = f"https://hacker-news.firebaseio.com/v0/item/{story_id}.json"
            try:
                resp = requests.get(item_url, timeout=FETCH_TIMEOUT, allow_redirects=False)
                resp.raise_for_status()
                story = resp.json()
            except (requests.RequestException, ValueError):
                continue

            if not story or story.get("type") != "story":
                continue

            title = normalize_unicode(story.get("title", ""))
            story_url = story.get("url", f"https://news.ycombinator.com/item?id={story_id}")

            published_at = None
            if story.get("time"):
                published_at = datetime.fromtimestamp(story["time"], tz=timezone.utc).isoformat()

            item = {
                "external_id": str(story_id),
                "title": title[:1000],
                "url": story_url[:2048],
                "summary": f"Score: {story.get('score', 0)} | Comments: {story.get('descendants', 0)}",
                "content": title[:100000],
                "author": story.get("by", ""),
                "published_at": published_at,
                "tags": tags[:20],
                "trust": trust,
                "extra": {
                    "score": story.get("score", 0),
                    "comments": story.get("descendants", 0),
                    "hn_url": f"https://news.ycombinator.com/item?id={story_id}",
                },
            }

            detected, patterns = detect_injection(item["title"])
            if detected:
                log_security_event("injection_in_feed", url=story_url, patterns=patterns)
                continue

            items.append(item)
            time.sleep(0.05)

        logger.info("Fetched %d items from HN %s", len(items), variant)
        return items

    def _fetch_reddit(self, variant: str, tags: list[str], trust: str, max_items: int, **kwargs) -> list[dict]:
        subreddit = kwargs.get("subreddit", "programming")
        if not subreddit:
            logger.warning("Reddit driver: no subreddit specified")
            return []

        subreddit = subreddit.strip().lstrip("r/")
        sort = kwargs.get("sort", variant or "hot")
        url = f"https://www.reddit.com/r/{subreddit}/{sort}.json?limit={max_items}"

        ok, reason = validate_url(url)
        if not ok:
            log_security_event("ssrf_blocked", url=url, reason=reason)
            return []

        try:
            resp = requests.get(
                url, timeout=FETCH_TIMEOUT,
                headers={"User-Agent": "vak-feeds/1.0 (feed aggregation)"},
                allow_redirects=False,
            )
            resp.raise_for_status()
            data = resp.json()
        except (requests.RequestException, ValueError) as e:
            logger.error("Failed to fetch Reddit r/%s: %s", subreddit, e)
            return []

        items = []
        for child in data.get("data", {}).get("children", [])[:max_items]:
            post = child.get("data", {})
            if post.get("stickied"):
                continue

            title = normalize_unicode(post.get("title", ""))
            post_url = post.get("url", "")
            permalink = f"https://reddit.com{post.get('permalink', '')}"

            published_at = None
            if post.get("created_utc"):
                published_at = datetime.fromtimestamp(post["created_utc"], tz=timezone.utc).isoformat()

            self_text = post.get("selftext", "")[:5000]

            item = {
                "external_id": post.get("id", "")[:256],
                "title": title[:1000],
                "url": post_url[:2048] if post_url else permalink[:2048],
                "summary": self_text[:10000] if self_text else title,
                "content": self_text[:100000] if self_text else title,
                "author": post.get("author", ""),
                "published_at": published_at,
                "tags": tags[:20],
                "trust": trust,
                "extra": {
                    "score": post.get("score", 0),
                    "comments": post.get("num_comments", 0),
                    "permalink": permalink,
                    "subreddit": subreddit,
                },
            }

            detected, patterns = detect_injection(item["title"] + " " + item["content"])
            if detected:
                log_security_event("injection_in_feed", url=post_url, patterns=patterns)
                continue

            items.append(item)

        logger.info("Fetched %d items from Reddit r/%s", len(items), subreddit)
        return items

    def _fetch_lobsters(self, variant: str, tags: list[str], trust: str, max_items: int, **kwargs) -> list[dict]:
        url = "https://lobste.rs/hottest.json"
        ok, reason = validate_url(url)
        if not ok:
            log_security_event("ssrf_blocked", url=url, reason=reason)
            return []

        try:
            resp = requests.get(url, timeout=FETCH_TIMEOUT,
                                headers={"User-Agent": "vak-feeds/1.0"}, allow_redirects=False)
            resp.raise_for_status()
            stories = resp.json()
        except (requests.RequestException, ValueError) as e:
            logger.error("Failed to fetch Lobsters: %s", e)
            return []

        items = []
        for story in stories[:max_items]:
            title = normalize_unicode(story.get("title", ""))
            story_url = story.get("url", "") or story.get("comments_url", "")

            published_at = None
            if story.get("created_at"):
                try:
                    published_at = datetime.fromisoformat(
                        story["created_at"].replace("Z", "+00:00")
                    ).isoformat()
                except (ValueError, TypeError):
                    pass

            story_tags = [t.get("tag", "") for t in story.get("tags", []) if t.get("tag")]

            item = {
                "external_id": str(story.get("short_id", ""))[:256],
                "title": title[:1000],
                "url": story_url[:2048],
                "summary": title[:10000],
                "content": title[:100000],
                "author": story.get("submitter_user", {}).get("username", "") if story.get("submitter_user") else "",
                "published_at": published_at,
                "tags": (story_tags + tags)[:20],
                "trust": trust,
                "extra": {
                    "score": story.get("score", 0),
                    "comments": story.get("comment_count", 0),
                    "comments_url": story.get("comments_url", ""),
                },
            }

            detected, patterns = detect_injection(item["title"])
            if detected:
                log_security_event("injection_in_feed", url=story_url, patterns=patterns)
                continue

            items.append(item)

        logger.info("Fetched %d items from Lobsters", len(items))
        return items

    def discover(self, query: str, driver: str = "hacker_news") -> list[dict]:
        """Discover aggregator sources matching the query."""
        results = []
        query_lower = query.lower()

        # Match against known aggregator variants
        if driver in ("hacker_news", "all"):
            variants = ["top", "new", "best", "ask", "show"]
            for v in variants:
                if not query or query_lower in v or "hn" in query_lower or "hacker" in query_lower:
                    results.append({
                        "name": f"Hacker News {v.title()}",
                        "type": "hacker_news",
                        "driver": "aggregator",
                        "variant": v,
                        "description": f"Hacker News {v} stories",
                    })

        if driver in ("reddit", "all"):
            # Common subreddits
            subreddits = [
                "programming", "technology", "machinelearning",
                "javascript", "python", "rust", "golang",
                "webdev", "devops", "opensource",
            ]
            for sub in subreddits:
                if not query or query_lower in sub:
                    results.append({
                        "name": f"r/{sub}",
                        "type": "reddit",
                        "driver": "aggregator",
                        "variant": sub,
                        "description": f"Reddit r/{sub} subreddit",
                    })

        if driver in ("lobsters", "all"):
            if not query or "lobster" in query_lower:
                results.append({
                    "name": "Lobste.rs Hot",
                    "type": "lobsters",
                    "driver": "aggregator",
                    "variant": "hot",
                    "description": "Lobste.rs hottest stories",
                })

        return results

    def preview(self, driver: str, variant: str = "top", limit: int = 5, **kwargs) -> list[dict]:
        items = self.fetch(driver, variant, max_items=limit, **kwargs)
        return [{"title": i["title"], "url": i["url"], "summary": i.get("summary", "")[:200]} for i in items]
