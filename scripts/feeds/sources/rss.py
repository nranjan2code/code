"""
RSS/Atom feed driver using feedparser.
"""

from __future__ import annotations

import logging
import time
from datetime import datetime, timezone
from typing import Any

import feedparser
import requests

from feed_security import detect_injection, log_security_event, normalize_unicode, validate_url

logger = logging.getLogger("feed_sources.rss")

FETCH_TIMEOUT = 15
MAX_REDIRECTS = 3
MAX_CONTENT_LENGTH = 512 * 1024


class RssDriver:
    source_type = "rss"

    def fetch(self, url: str, tags: list[str] | None = None, trust: str = "medium",
              max_items: int = 50, **kwargs) -> list[dict]:
        if not url:
            logger.warning("RSS driver: no URL provided")
            return []

        ok, reason = validate_url(url)
        if not ok:
            logger.error("RSS URL validation failed: %s — %s", url, reason)
            log_security_event("ssrf_blocked", url=url, reason=reason)
            return []

        try:
            resp = requests.get(
                url,
                timeout=FETCH_TIMEOUT,
                headers={"User-Agent": "vak-feeds/1.0 (+https://github.com/vakcoder)"},
                allow_redirects=False,
            )
            resp.raise_for_status()

            if len(resp.content) > MAX_CONTENT_LENGTH:
                logger.warning("RSS feed too large: %d bytes", len(resp.content))
                return []

        except requests.RequestException as e:
            logger.error("Failed to fetch RSS feed %s: %s", url, e)
            return []

        feed = feedparser.parse(resp.content)
        items = []

        for entry in feed.entries[:max_items]:
            title = normalize_unicode(entry.get("title", ""))
            link = entry.get("link", "")
            summary = _clean_html(entry.get("summary", entry.get("description", "")))
            content_text = ""
            if entry.get("content"):
                content_text = _clean_html(entry.content[0].get("value", ""))

            published = entry.get("published_parsed") or entry.get("updated_parsed")
            published_at = None
            if published:
                try:
                    published_at = datetime(*published[:6], tzinfo=timezone.utc).isoformat()
                except (ValueError, TypeError):
                    pass

            author = entry.get("author", "")

            entry_tags = []
            if hasattr(entry, "tags"):
                entry_tags = [t.get("term", "") for t in entry.tags if t.get("term")]

            external_id = entry.get("id", entry.get("guid", link))

            item = {
                "external_id": external_id[:256] if external_id else link[:256],
                "title": title[:1000],
                "url": link[:2048] if link else url[:2048],
                "summary": summary[:10000],
                "content": content_text[:100000] if content_text else summary[:100000],
                "author": author[:500],
                "published_at": published_at,
                "tags": (entry_tags + (tags or []))[:20],
                "trust": trust,
            }

            detected, patterns = detect_injection(item["title"] + " " + item["content"])
            if detected:
                log_security_event("injection_in_feed", url=url, patterns=patterns)
                continue

            items.append(item)

        logger.info("Fetched %d items from RSS: %s", len(items), url)
        return items

    def discover(self, url: str) -> list[dict]:
        ok, reason = validate_url(url)
        if not ok:
            return []

        try:
            resp = requests.get(
                url, timeout=FETCH_TIMEOUT,
                headers={"User-Agent": "vak-feeds/1.0"},
                allow_redirects=False,
            )
            resp.raise_for_status()
        except requests.RequestException:
            return []

        feeds = []
        text = resp.text.lower()
        import re
        for match in re.finditer(
            r'<link[^>]+(?:type|itemprop)=["\'](?:application/(?:rss|atom)\+xml|feed)["\'][^>]*>',
            text,
        ):
            tag = match.group(0)
            href_match = re.search(r'href=["\']([^"\']+)["\']', tag)
            title_match = re.search(r'title=["\']([^"\']+)["\']', tag)
            if href_match:
                feed_url = href_match.group(1)
                if not feed_url.startswith("http"):
                    from urllib.parse import urljoin
                    feed_url = urljoin(url, feed_url)
                feeds.append({
                    "url": feed_url,
                    "title": title_match.group(1) if title_match else feed_url,
                })

        if not feeds:
            for match in re.finditer(r'href=["\']([^"\']*(?:rss|feed|atom)[^"\']*)["\']', text):
                feed_url = match.group(1)
                if not feed_url.startswith("http"):
                    from urllib.parse import urljoin
                    feed_url = urljoin(url, feed_url)
                feeds.append({"url": feed_url, "title": feed_url})

        return feeds

    def preview(self, url: str, limit: int = 5) -> list[dict]:
        items = self.fetch(url, max_items=limit)
        return [{"title": i["title"], "url": i["url"], "summary": i["summary"][:200]} for i in items]


def _clean_html(html: str) -> str:
    import nh3
    return nh3.clean_text(html) if html else ""
