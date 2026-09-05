"""
YouTube channel RSS driver.
Uses YouTube's public RSS feed endpoint — no API key required.
"""

from __future__ import annotations

import logging
import re
import xml.etree.ElementTree as ET
from datetime import datetime, timezone
from typing import Any

import requests

from feed_security import detect_injection, log_security_event, normalize_unicode, sanitize_html, validate_url

logger = logging.getLogger("feed_sources.youtube")

FETCH_TIMEOUT = 15
MAX_ITEMS = 50


class YouTubeDriver:
    source_type = "youtube"

    def _normalize_channel_id(self, channel_input: str) -> str | None:
        channel_input = channel_input.strip()
        match = re.match(r'(?:https?://)?(?:www\.)?youtube\.com/(?:channel/|c/|@)([^/?&#]+)', channel_input)
        if match:
            return match.group(1)
        if re.match(r'^UC[a-zA-Z0-9_-]{22}$', channel_input):
            return channel_input
        return channel_input

    def _resolve_channel_id(self, handle: str) -> str | None:
        if re.match(r'^UC[a-zA-Z0-9_-]{22}$', handle):
            return handle

        url = f"https://www.youtube.com/{handle}"
        ok, reason = validate_url(url)
        if not ok:
            return None

        try:
            resp = requests.get(url, timeout=FETCH_TIMEOUT,
                                headers={"User-Agent": "vak-feeds/1.0"}, allow_redirects=False)
            resp.raise_for_status()
            match = re.search(r'"externalId"\s*:\s*"(UC[a-zA-Z0-9_-]{22})"', resp.text)
            if match:
                return match.group(1)
        except requests.RequestException as e:
            logger.error("Failed to resolve channel %s: %s", handle, e)
        return None

    def fetch(self, channel_id: str, tags: list[str] | None = None, trust: str = "medium",
              max_items: int = MAX_ITEMS, **kwargs) -> list[dict]:
        resolved = self._resolve_channel_id(channel_id)
        if not resolved:
            logger.error("Could not resolve YouTube channel: %s", channel_id)
            return []

        rss_url = f"https://www.youtube.com/feeds/videos.xml?channel_id={resolved}"
        ok, reason = validate_url(rss_url)
        if not ok:
            log_security_event("ssrf_blocked", url=rss_url, reason=reason)
            return []

        try:
            resp = requests.get(rss_url, timeout=FETCH_TIMEOUT,
                                headers={"User-Agent": "vak-feeds/1.0"}, allow_redirects=False)
            resp.raise_for_status()
        except requests.RequestException as e:
            logger.error("Failed to fetch YouTube RSS for %s: %s", resolved, e)
            return []

        return self._parse_feed(resp.text, tags or [], trust, max_items)

    def _parse_feed(self, xml_text: str, tags: list[str], trust: str, max_items: int) -> list[dict]:
        items = []
        try:
            root = ET.fromstring(xml_text)
        except ET.ParseError as e:
            logger.error("Failed to parse YouTube XML: %s", e)
            return []

        ns = {"atom": "http://www.w3.org/2005/Atom", "media": "http://search.yahoo.com/mrss/"}

        for entry in root.findall("atom:entry", ns)[:max_items]:
            title_el = entry.find("atom:title", ns)
            title = normalize_unicode(title_el.text if title_el is not None else "")

            link_el = entry.find("atom:link", ns)
            url = link_el.get("href", "") if link_el is not None else ""

            published_el = entry.find("atom:published", ns)
            published_at = None
            if published_el is not None and published_el.text:
                try:
                    dt = datetime.fromisoformat(published_el.text.replace("Z", "+00:00"))
                    published_at = dt.isoformat()
                except (ValueError, TypeError):
                    pass

            author_el = entry.find("atom:author/atom:name", ns)
            author = author_el.text if author_el is not None else ""

            group_el = entry.find("media:group", ns)
            description = ""
            if group_el is not None:
                desc_el = group_el.find("media:description", ns)
                description = sanitize_html(desc_el.text if desc_el is not None else "")

            thumbnail_url = ""
            if group_el is not None:
                thumb_el = group_el.find("media:thumbnail", ns)
                if thumb_el is not None:
                    thumbnail_url = thumb_el.get("url", "")

            video_id_el = entry.find("atom:id", ns)
            video_id = video_id_el.text if video_id_el is not None else url

            item = {
                "external_id": video_id[:256] if video_id else url[:256],
                "title": title[:1000],
                "url": url[:2048],
                "summary": description[:10000],
                "content": description[:100000],
                "author": (author or "")[:500],
                "published_at": published_at,
                "tags": tags[:20],
                "trust": trust,
            }

            if thumbnail_url:
                item["extra"] = {"thumbnail": thumbnail_url}

            detected, patterns = detect_injection(item["title"] + " " + item["content"])
            if detected:
                log_security_event("injection_in_feed", url=url, patterns=patterns)
                continue

            items.append(item)

        logger.info("Fetched %d items from YouTube", len(items))
        return items

    def discover(self, query: str) -> list[dict]:
        url = f"https://www.youtube.com/results?search_query={query}"
        ok, reason = validate_url(url)
        if not ok:
            return []

        try:
            resp = requests.get(url, timeout=FETCH_TIMEOUT,
                                headers={"User-Agent": "vak-feeds/1.0"}, allow_redirects=False)
            resp.raise_for_status()
        except requests.RequestException:
            return []

        channels = []
        for match in re.finditer(
            r'"channelId"\s*:\s*"(UC[a-zA-Z0-9_-]{22})".*?"title"\s*:\s*\{"runs":\[\{"text"\s*:\s*"([^"]+)"\}',
            resp.text,
        ):
            channels.append({
                "channel_id": match.group(1),
                "name": match.group(2),
                "url": f"https://youtube.com/channel/{match.group(1)}",
            })

        if not channels:
            for match in re.finditer(r'"channelId"\s*:\s*"(UC[a-zA-Z0-9_-]{22})"', resp.text):
                cid = match.group(1)
                if not any(c["channel_id"] == cid for c in channels):
                    channels.append({
                        "channel_id": cid,
                        "name": cid,
                        "url": f"https://youtube.com/channel/{cid}",
                    })

        return channels[:10]

    def preview(self, channel_id: str, limit: int = 5) -> list[dict]:
        items = self.fetch(channel_id, max_items=limit)
        return [{"title": i["title"], "url": i["url"], "summary": i["summary"][:200]} for i in items]
