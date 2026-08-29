"""
Custom HTTP source driver.
Supports JSON, RSS, and text responses from any HTTP endpoint.
"""

from __future__ import annotations

import json
import logging
import re
from datetime import datetime, timezone
from typing import Any

import feedparser
import requests

from feed_security import detect_injection, log_security_event, normalize_unicode, sanitize_html, validate_url

logger = logging.getLogger("feed_sources.custom")

FETCH_TIMEOUT = 15
MAX_CONTENT_LENGTH = 512 * 1024


class CustomHttpDriver:
    source_type = "custom"

    def fetch(self, url: str, format: str = "json", json_path: str = "",
              mapping: dict[str, str] | None = None, tags: list[str] | None = None,
              trust: str = "medium", max_items: int = 50, **kwargs) -> list[dict]:
        if not url:
            logger.warning("Custom driver: no URL provided")
            return []

        ok, reason = validate_url(url)
        if not ok:
            log_security_event("ssrf_blocked", url=url, reason=reason)
            return []

        try:
            resp = requests.get(
                url, timeout=FETCH_TIMEOUT,
                headers={"User-Agent": "vak-feeds/1.0"},
            )
            resp.raise_for_status()
        except requests.RequestException as e:
            logger.error("Failed to fetch custom source %s: %s", url, e)
            return []

        if len(resp.content) > MAX_CONTENT_LENGTH:
            logger.warning("Custom source response too large: %d bytes", len(resp.content))
            return []

        if format == "json":
            return self._parse_json(resp, json_path, mapping or {}, tags or [], trust, max_items, url)
        elif format in ("rss", "atom"):
            return self._parse_rss(resp, tags or [], trust, max_items, url)
        elif format == "text":
            return self._parse_text(resp, tags or [], trust, max_items, url)
        else:
            logger.error("Unknown custom format: %s", format)
            return []

    def _parse_json(self, resp: requests.Response, json_path: str,
                    mapping: dict[str, str], tags: list[str], trust: str,
                    max_items: int, source_url: str) -> list[dict]:
        try:
            data = resp.json()
        except json.JSONDecodeError as e:
            logger.error("Failed to parse JSON from %s: %s", source_url, e)
            return []

        if json_path:
            for key in json_path.split("."):
                if isinstance(data, dict) and key in data:
                    data = data[key]
                else:
                    logger.error("JSON path '%s' not found in response", json_path)
                    return []

        if not isinstance(data, list):
            data = [data]

        items = []
        for raw_item in data[:max_items]:
            if not isinstance(raw_item, dict):
                continue

            item = {
                "external_id": str(raw_item.get(mapping.get("id", "id"), ""))[:256],
                "title": normalize_unicode(str(raw_item.get(mapping.get("title", "title"), "")))[:1000],
                "url": str(raw_item.get(mapping.get("url", "url"), ""))[:2048],
                "summary": sanitize_html(str(raw_item.get(mapping.get("summary", "summary"), "")))[:10000],
                "content": sanitize_html(str(raw_item.get(mapping.get("content", "content"), "")))[:100000],
                "author": str(raw_item.get(mapping.get("author", "author"), ""))[:500],
                "published_at": self._parse_date(raw_item.get(mapping.get("published_at", "published_at"))),
                "tags": tags[:20],
                "trust": trust,
            }

            if not item["external_id"]:
                item["external_id"] = item["url"][:256] if item["url"] else ""

            detected, patterns = detect_injection(item["title"] + " " + item["content"])
            if detected:
                log_security_event("injection_in_feed", url=source_url, patterns=patterns)
                continue

            items.append(item)

        logger.info("Fetched %d items from custom JSON: %s", len(items), source_url)
        return items

    def _parse_rss(self, resp: requests.Response, tags: list[str], trust: str,
                   max_items: int, source_url: str) -> list[dict]:
        feed = feedparser.parse(resp.content)
        items = []

        for entry in feed.entries[:max_items]:
            title = normalize_unicode(entry.get("title", ""))
            link = entry.get("link", "")
            summary = sanitize_html(entry.get("summary", ""))

            published = entry.get("published_parsed") or entry.get("updated_parsed")
            published_at = None
            if published:
                try:
                    published_at = datetime(*published[:6], tzinfo=timezone.utc).isoformat()
                except (ValueError, TypeError):
                    pass

            item = {
                "external_id": entry.get("id", link)[:256],
                "title": title[:1000],
                "url": link[:2048],
                "summary": summary[:10000],
                "content": summary[:100000],
                "author": entry.get("author", "")[:500],
                "published_at": published_at,
                "tags": tags[:20],
                "trust": trust,
            }

            detected, patterns = detect_injection(item["title"] + " " + item["content"])
            if detected:
                log_security_event("injection_in_feed", url=source_url, patterns=patterns)
                continue

            items.append(item)

        logger.info("Fetched %d items from custom RSS: %s", len(items), source_url)
        return items

    def _parse_text(self, resp: requests.Response, tags: list[str], trust: str,
                    max_items: int, source_url: str) -> list[dict]:
        lines = resp.text.strip().split("\n")
        items = []

        for i, line in enumerate(lines[:max_items]):
            line = line.strip()
            if not line:
                continue

            item = {
                "external_id": f"{source_url}#{i}"[:256],
                "title": normalize_unicode(line[:1000]),
                "url": source_url[:2048],
                "summary": line[:10000],
                "content": line[:100000],
                "author": "",
                "published_at": datetime.now(timezone.utc).isoformat(),
                "tags": tags[:20],
                "trust": trust,
            }

            detected, patterns = detect_injection(line)
            if detected:
                log_security_event("injection_in_feed", url=source_url, patterns=patterns)
                continue

            items.append(item)

        logger.info("Fetched %d items from custom text: %s", len(items), source_url)
        return items

    def _parse_date(self, value: Any) -> str | None:
        if not value:
            return None
        if isinstance(value, (int, float)):
            try:
                return datetime.fromtimestamp(value, tz=timezone.utc).isoformat()
            except (ValueError, OSError):
                return None
        if isinstance(value, str):
            for fmt in ("%Y-%m-%dT%H:%M:%S%z", "%Y-%m-%dT%H:%M:%SZ", "%Y-%m-%d", "%Y-%m-%dT%H:%M:%S"):
                try:
                    return datetime.strptime(value, fmt).replace(tzinfo=timezone.utc).isoformat()
                except ValueError:
                    continue
        return None

    def discover(self, url: str) -> list[dict]:
        ok, reason = validate_url(url)
        if not ok:
            return []

        try:
            resp = requests.get(url, timeout=FETCH_TIMEOUT,
                                headers={"User-Agent": "vak-feeds/1.0"})
            resp.raise_for_status()
        except requests.RequestException:
            return []

        content_type = resp.headers.get("content-type", "")
        format = "text"
        if "json" in content_type:
            format = "json"
        elif "xml" in content_type or "rss" in content_type or "atom" in content_type:
            format = "rss"

        return [{"url": url, "format": format, "title": url}]

    def probe(self, url: str) -> dict:
        ok, reason = validate_url(url)
        if not ok:
            return {"error": reason}

        try:
            resp = requests.get(url, timeout=FETCH_TIMEOUT,
                                headers={"User-Agent": "vak-feeds/1.0"})
            resp.raise_for_status()
        except requests.RequestException as e:
            return {"error": str(e)}

        content_type = resp.headers.get("content-type", "")
        format = "text"
        if "json" in content_type:
            format = "json"
        elif "xml" in content_type or "rss" in content_type:
            format = "rss"

        sample = resp.text[:500]
        return {
            "format": format,
            "content_type": content_type,
            "sample": sample,
            "status_code": resp.status_code,
        }
