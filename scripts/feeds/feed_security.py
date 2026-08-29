"""
feed_security.py — Security layer for the feed pipeline.

Provides: SSRF prevention, HTML sanitization, Unicode normalization,
injection detection, schema validation, and safe query construction.
"""

from __future__ import annotations

import ipaddress
import json
import logging
import re
import socket
import unicodedata
from datetime import datetime
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

logger = logging.getLogger("feed_security")

# ─── SSRF Prevention ───

BLOCKED_IP_RANGES = [
    ipaddress.ip_network("10.0.0.0/8"),
    ipaddress.ip_network("172.16.0.0/12"),
    ipaddress.ip_network("192.168.0.0/16"),
    ipaddress.ip_network("127.0.0.0/8"),
    ipaddress.ip_network("169.254.0.0/16"),
    ipaddress.ip_network("0.0.0.0/8"),
    ipaddress.ip_network("::1/128"),
    ipaddress.ip_network("fc00::/7"),
    ipaddress.ip_network("fe80::/10"),
    ipaddress.ip_network("224.0.0.0/4"),
    ipaddress.ip_network("100.64.0.0/10"),
    ipaddress.ip_network("192.0.0.0/24"),
    ipaddress.ip_network("192.0.2.0/24"),
    ipaddress.ip_network("198.51.100.0/24"),
    ipaddress.ip_network("203.0.113.0/24"),
    ipaddress.ip_network("240.0.0.0/4"),
]

ALLOWED_SCHEMES = {"http", "https"}

DNS_REBIND_CACHE: dict[str, tuple[str, datetime]] = {}
DNS_CACHE_TTL_SECONDS = 300


def is_blocked_ip(ip_str: str) -> bool:
    try:
        ip = ipaddress.ip_address(ip_str)
        return any(ip in net for net in BLOCKED_IP_RANGES)
    except ValueError:
        return True


def validate_url(url: str) -> tuple[bool, str]:
    try:
        parsed = urlparse(url)
    except Exception as e:
        return False, f"URL parse error: {e}"

    if parsed.scheme not in ALLOWED_SCHEMES:
        return False, f"Blocked scheme: {parsed.scheme}"

    hostname = parsed.hostname
    if not hostname:
        return False, "No hostname"

    if len(url) > 2048:
        return False, "URL exceeds 2048 characters"

    try:
        addrinfos = socket.getaddrinfo(hostname, None, socket.AF_UNSPEC, socket.SOCK_STREAM)
        if not addrinfos:
            return False, "DNS resolution returned no results"

        resolved_ips = set()
        for family, _, _, _, sockaddr in addrinfos:
            ip = str(ipaddress.ip_address(sockaddr[0]))
            resolved_ips.add(ip)
            if is_blocked_ip(ip):
                return False, f"Blocked IP: {ip}"

        cache_key = hostname
        now = datetime.utcnow()
        if cache_key in DNS_REBIND_CACHE:
            cached_ip, cached_time = DNS_REBIND_CACHE[cache_key]
            if (now - cached_time).total_seconds() < DNS_CACHE_TTL_SECONDS:
                if not resolved_ips.intersection({cached_ip}):
                    log_security_event("dns_rebind_detected", hostname=hostname,
                                       old_ip=cached_ip, new_ips=list(resolved_ips))
                    return False, f"DNS rebinding detected for {hostname}"

        DNS_REBIND_CACHE[cache_key] = (list(resolved_ips)[0], now)

    except socket.gaierror as e:
        return False, f"DNS resolution failed: {e}"
    except Exception as e:
        return False, f"URL validation error: {e}"

    return True, "ok"


# ─── Unicode Normalization ───

ZERO_WIDTH_RE = re.compile(
    '[\u200b\u200c\u200d\u200e\u200f'
    '\u2028\u2029\u202a\u202b\u202c\u202d\u202e'
    '\u2060\u2061\u2062\u2063\u2064'
    '\ufeff\u00ad]'
)

TAG_CHARS_RE = re.compile('[\ue000-\ue07f]')

HOMOGLYPH_MAP: dict[str, str] = {
    '\u0456': 'i', '\u0430': 'a', '\u0435': 'e', '\u043e': 'o',
    '\u0440': 'p', '\u0441': 'c', '\u0443': 'y', '\u0445': 'x',
    '\u0491': 'r', '\u045e': 'u', '\u0455': 's',
    '\u03b1': 'a', '\u03b5': 'e', '\u03bf': 'o', '\u03c0': 'p',
    '\u03c1': 'p', '\u03c3': 's', '\u03c4': 't', '\u03c5': 'u',
    '\uff10': '0', '\uff11': '1', '\uff12': '2', '\uff13': '3',
    '\uff14': '4', '\uff15': '5', '\uff16': '6', '\uff17': '7',
    '\uff18': '8', '\uff19': '9',
    '\uff21': 'A', '\uff22': 'B', '\uff23': 'C', '\uff24': 'D',
    '\uff25': 'E', '\uff26': 'F', '\uff27': 'G', '\uff28': 'H',
    '\uff29': 'I', '\uff2a': 'J', '\uff2b': 'K', '\uff2c': 'L',
    '\uff2d': 'M', '\uff2e': 'N', '\uff2f': 'O', '\uff30': 'P',
    '\uff31': 'Q', '\uff32': 'R', '\uff33': 'S', '\uff34': 'T',
    '\uff35': 'U', '\uff36': 'V', '\uff37': 'W', '\uff38': 'X',
    '\uff39': 'Y', '\uff3a': 'Z',
}


def normalize_unicode(text: str) -> str:
    text = unicodedata.normalize('NFKC', text)
    text = ZERO_WIDTH_RE.sub('', text)
    text = TAG_CHARS_RE.sub('', text)
    text = ''.join(HOMOGLYPH_MAP.get(c, c) for c in text)
    return text


# ─── HTML Sanitization ───

def sanitize_html(text: str, strip_all: bool = True) -> str:
    import html
    import re
    # Decode HTML entities first
    text = html.unescape(text)
    if strip_all:
        # Strip all HTML tags, keeping only text content
        text = re.sub(r'<[^>]+>', '', text)
        # Normalize whitespace
        text = re.sub(r'\s+', ' ', text).strip()
        return text
    import nh3
    return nh3.clean(
        text,
        tags={"p", "b", "i", "a", "ul", "ol", "li", "code", "pre", "em", "strong", "br"},
        attributes={"a": {"href"}},
        url_schemes={"http", "https", "mailto"},
    )


# ─── Injection Detection ───

INJECTION_PATTERNS = [
    r'ignore\s+(all\s+)?previous\s+instructions',
    r'ignore\s+(all\s+)?prior\s+instructions',
    r'disregard\s+(all\s+)?previous',
    r'forget\s+(all\s+)?previous',
    r'override\s+(all\s+)?instructions',
    r'you\s+are\s+now\s+(in\s+)?developer\s+mode',
    r'you\s+are\s+now\s+a\s+new',
    r'act\s+as\s+if\s+you\s+have\s+no',
    r'pretend\s+you\s+are\s+',
    r'system\s*:\s*',
    r'assistant\s*:\s*',
    r'\[INST\]',
    r'<<SYS>>',
    r'</?s>',
    r'reveal\s+(your\s+)?(system\s+)?prompt',
    r'what\s+(are|is)\s+your\s+instructions',
    r'show\s+me\s+your\s+system\s+prompt',
    r'output\s+your\s+instructions',
    r'send\s+(this|all|the)\s+(data|info|information)\s+to',
    r'email\s+(the|this|all)\s+to',
    r'upload\s+(this|all)\s+to\s+http',
    r'exec\s*\(',
    r'eval\s*\(',
    r'__import__',
    r'os\.system',
    r'subprocess\.(call|run|Popen)',
    r'<script[\s>]',
    r'on(error|load|click|mouse)\s*=',
    r'javascript\s*:',
    r'!\[.*\]\(http',
    r'<iframe',
    r'<object',
    r'<embed',
]

_injection_re = re.compile('|'.join(INJECTION_PATTERNS), re.IGNORECASE)


def detect_injection(text: str) -> tuple[bool, list[str]]:
    normalized = normalize_unicode(text)
    matches = _injection_re.findall(normalized)
    if matches:
        unique = list(set(m if isinstance(m, str) else m[0] for m in matches))
        return True, unique
    return False, []


# ─── Content Fingerprinting (for dedup) ───

def content_fingerprint(title: str, url: str) -> str:
    import hashlib
    raw = f"{normalize_unicode(title.lower().strip())}|{url.strip()}"
    return hashlib.sha256(raw.encode()).hexdigest()


# ─── Schema Validation ───

try:
    from pydantic import BaseModel, Field, field_validator
    HAS_PYDANTIC = True
except ImportError:
    HAS_PYDANTIC = False

if HAS_PYDANTIC:
    from typing import Optional

    class SecureFeedItem(BaseModel):
        external_id: str = Field(..., max_length=256)
        title: str = Field(..., max_length=1000)
        url: str = Field(..., max_length=2048)
        summary: str = Field(default="", max_length=10000)
        content: str = Field(default="", max_length=100000)
        author: str = Field(default="", max_length=500)
        published_at: Optional[str] = None
        tags: list[str] = Field(default_factory=list)

        @field_validator('title', 'summary', 'author')
        @classmethod
        def sanitize_text_fields(cls, v: str) -> str:
            v = normalize_unicode(v)
            v = sanitize_html(v, strip_all=True)
            return v

        @field_validator('content')
        @classmethod
        def sanitize_content(cls, v: str) -> str:
            v = normalize_unicode(v)
            v = sanitize_html(v, strip_all=True)
            return v

        @field_validator('url')
        @classmethod
        def validate_url_safety(cls, v: str) -> str:
            ok, reason = validate_url(v)
            if not ok:
                raise ValueError(f"URL validation failed: {reason}")
            return v

        @field_validator('tags')
        @classmethod
        def sanitize_tags(cls, v: list[str]) -> list[str]:
            return [normalize_unicode(t.lower().strip())[:50] for t in v[:20]]


# ─── MCP Response Sanitization ───

def sanitize_for_llm(text: str, max_length: int = 10000) -> str:
    text = normalize_unicode(text)
    text = sanitize_html(text, strip_all=True)
    text = text[:max_length]
    detected, patterns = detect_injection(text)
    if detected:
        return f"[Content filtered: {len(patterns)} suspicious patterns detected]"
    return text


def sanitize_mcp_response(response: Any) -> Any:
    if isinstance(response, dict):
        return {k: sanitize_mcp_response(v) for k, v in response.items()}
    if isinstance(response, list):
        return [sanitize_mcp_response(item) for item in response]
    if isinstance(response, str):
        return sanitize_for_llm(response)
    return response


# ─── Security Event Logging ───

_security_log_path: Path | None = None


def init_security_log(data_home: str | Path) -> None:
    global _security_log_path
    log_dir = Path(data_home) / "feeds"
    log_dir.mkdir(parents=True, exist_ok=True)
    _security_log_path = log_dir / "security.log"


def log_security_event(event_type: str, **context: Any) -> None:
    entry = {
        "timestamp": datetime.utcnow().isoformat(),
        "event": event_type,
        **{k: str(v)[:500] for k, v in context.items()},
    }
    if _security_log_path:
        try:
            with open(_security_log_path, "a") as f:
                f.write(json.dumps(entry) + "\n")
        except OSError:
            pass
    logger.warning("SECURITY: %s %s", event_type, context)
