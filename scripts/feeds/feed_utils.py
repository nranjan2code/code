"""
feed_utils.py — Config loading, DuckDB helpers, dedup, and shared utilities.
"""

from __future__ import annotations

import json
import logging
import os
import sys
import time
import tomllib
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Optional

from feed_security import (
    content_fingerprint,
    detect_injection,
    init_security_log,
    log_security_event,
    normalize_unicode,
    sanitize_html,
    validate_url,
)

logger = logging.getLogger("feed_utils")

# ─── Path Resolution ───

def get_data_home() -> Path:
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "vak"
    return Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")) / "vak"


def get_cache_home() -> Path:
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Caches" / "vak"
    return Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "vak"


def feeds_dir() -> Path:
    d = get_data_home() / "feeds"
    d.mkdir(parents=True, exist_ok=True)
    return d


def db_path() -> Path:
    return feeds_dir() / "feeds.duckdb"


# ─── Config Types ───

@dataclass
class FeedSourceConfig:
    id: str
    name: str
    source_type: str
    url: str = ""
    driver: str = ""
    channel_id: str = ""
    variant: str = ""
    tags: list[str] = field(default_factory=list)
    trust: str = "medium"
    enabled: bool = True
    interval: str = "1h"
    format: str = "rss"
    json_path: str = ""
    mapping: dict[str, str] = field(default_factory=dict)
    sort: str = "hot"
    extra: dict[str, Any] = field(default_factory=dict)


@dataclass
class AlertConfig:
    id: int = 0
    name: str = ""
    keywords: list[str] = field(default_factory=list)
    tags: list[str] = field(default_factory=list)
    sources: list[str] = field(default_factory=list)
    action: str = "deliver"
    deliver_to: str = ""
    hook_command: str = ""
    cooldown_minutes: int = 30
    enabled: bool = True


@dataclass
class FeedConfig:
    default_check_interval: str = "30m"
    max_items_per_feed: int = 500
    dedup_window_days: int = 90
    sources: list[FeedSourceConfig] = field(default_factory=list)
    alerts: list[AlertConfig] = field(default_factory=list)


# ─── Config Loading ───

def parse_interval(interval: str) -> int:
    interval = interval.strip().lower()
    multipliers = {"s": 1, "m": 60, "h": 3600, "d": 86400}
    if interval[-1] in multipliers:
        return int(interval[:-1]) * multipliers[interval[-1]]
    return int(interval) * 60


def load_toml(path: Path) -> dict:
    if not path.exists():
        return {}
    with open(path, "rb") as f:
        return tomllib.load(f)


def load_feed_config(workspace: str | Path | None = None) -> FeedConfig:
    global_config_path = Path.home() / ".config" / "vak" / "feeds.toml"
    global_data = load_toml(global_config_path)

    workspace_config_path = None
    if workspace:
        ws = Path(workspace)
        workspace_config_path = ws / ".vak" / "feeds.toml"
    workspace_data = load_toml(workspace_config_path) if workspace_config_path else {}

    general = {**global_data.get("general", {}), **workspace_data.get("general", {})}
    sources_raw = global_data.get("sources", []) + workspace_data.get("sources", [])
    alerts_raw = global_data.get("alerts", []) + workspace_data.get("alerts", [])

    sources = []
    seen_names: set[str] = set()
    for s in sources_raw:
        name = s.get("name", "")
        if name in seen_names:
            continue
        seen_names.add(name)
        if not s.get("enabled", True):
            continue
        sources.append(FeedSourceConfig(
            id=s.get("id", name.lower().replace(" ", "-")),
            name=name,
            source_type=s.get("type", "rss"),
            url=s.get("url", ""),
            driver=s.get("driver", ""),
            channel_id=s.get("channel_id", ""),
            variant=s.get("variant", ""),
            tags=s.get("tags", []),
            trust=s.get("trust", "medium"),
            enabled=s.get("enabled", True),
            interval=s.get("interval", general.get("default_check_interval", "1h")),
            format=s.get("format", "rss"),
            json_path=s.get("json_path", ""),
            mapping=s.get("mapping", {}),
            sort=s.get("sort", "hot"),
            extra={k: v for k, v in s.items() if k not in (
                "id", "name", "type", "url", "driver", "channel_id", "variant",
                "tags", "trust", "enabled", "interval", "format", "json_path", "mapping", "sort",
            )},
        ))

    alerts = []
    for a in alerts_raw:
        match = a.get("match", {})
        alerts.append(AlertConfig(
            name=a.get("name", ""),
            keywords=match.get("keywords", []),
            tags=match.get("tags", []),
            sources=match.get("sources", []),
            action=a.get("action", "deliver"),
            deliver_to=a.get("deliver_to", ""),
            hook_command=a.get("hook_command", ""),
            cooldown_minutes=a.get("cooldown_minutes", 30),
            enabled=a.get("enabled", True),
        ))

    return FeedConfig(
        default_check_interval=general.get("default_check_interval", "30m"),
        max_items_per_feed=general.get("max_items_per_feed", 500),
        dedup_window_days=general.get("dedup_window_days", 90),
        sources=sources,
        alerts=alerts,
    )


def load_source_registry(workspace: str | Path | None = None) -> dict[str, dict]:
    registry_path = Path(__file__).parent / "feed_sources.toml"
    user_registry = Path.home() / ".config" / "vak" / "feed_sources.toml"

    registry_data = load_toml(registry_path)
    if user_registry.exists():
        user_data = load_toml(user_registry)
        registry_data.setdefault("source_types", [])
        existing_ids = {st["id"] for st in registry_data.get("source_types", [])}
        for st in user_data.get("source_types", []):
            if st.get("id") not in existing_ids:
                registry_data["source_types"].append(st)

    return {st["id"]: st for st in registry_data.get("source_types", [])}


# ─── DuckDB Helpers ───
#
# DuckDB allows either multiple concurrent read-only connections, or a
# single read-write connection -- never both. The admin UI fires several
# read endpoints (stats, sources, alerts) as parallel HTTP requests, each
# of which spawns its own python3 subprocess; when those raced to open
# get_db()'s old always-read-write connection, one lost the exclusive
# lock and the request 500'd (a real, reproducible failure -- not a flake
# to shrug off). Pure readers now open read_only=True so they can freely
# overlap with each other; writers still take the exclusive lock, but any
# caller that loses a race retries with backoff instead of failing hard.

_LOCK_RETRY_ATTEMPTS = 5
_LOCK_RETRY_BASE_DELAY_S = 0.05  # 50ms, 100ms, 200ms, 400ms, 800ms


def get_db(read_only: bool = False):
    import duckdb

    path = db_path()
    path.parent.mkdir(parents=True, exist_ok=True)

    # A read-only open of a file that doesn't exist yet raises in DuckDB;
    # fall back to a normal (write) connection so the caller gets a valid,
    # empty database instead of an IOException. init_feed_system() always
    # runs init_db() before any reader in this pipeline is reachable, so
    # in practice this only matters for callers used outside that flow.
    effective_read_only = read_only and path.exists()

    # Every mode retries: two readers never conflict with each other, but
    # a reader can still land while a short-lived writer (an ingest run)
    # holds the exclusive lock, and two writers can race the same way the
    # bug report did before reads were split out. The holder in both
    # cases is a subprocess that opens, does its work, and closes within
    # milliseconds, so a short backoff clears almost every real conflict
    # instead of surfacing it as a 500.
    last_err: Exception | None = None
    for attempt in range(_LOCK_RETRY_ATTEMPTS):
        try:
            con = duckdb.connect(str(path), read_only=effective_read_only)
            con.execute("SET enable_progress_bar = false")
            return con
        except duckdb.Error as e:
            last_err = e
            if attempt == _LOCK_RETRY_ATTEMPTS - 1:
                raise
            time.sleep(_LOCK_RETRY_BASE_DELAY_S * (2 ** attempt))
    raise last_err  # pragma: no cover — loop always returns or raises


def _schema_exists() -> bool:
    """Cheap read-only check for whether init.sql has already run.

    Every script in this pipeline calls init_feed_system() -> init_db() on
    startup, including the pure-read paths (a single admin-UI page load
    spawns three separate subprocesses this way). Re-running "CREATE TABLE
    IF NOT EXISTS ..." is a no-op once the schema exists, but doing it via
    a write connection still takes the exclusive lock every time -- which
    is most of what was left contending after get_db() split off read-only
    readers. Checking first via a read-only connection lets a fully-warm
    database skip the write path entirely.
    """
    import duckdb

    if not db_path().exists():
        return False
    try:
        con = get_db(read_only=True)
    except duckdb.Error:
        return False
    try:
        con.execute("SELECT 1 FROM feeds LIMIT 0")
        return True
    except duckdb.Error:
        return False
    finally:
        con.close()


def init_db() -> None:
    import duckdb

    if _schema_exists():
        return

    schema_path = Path(__file__).parent / "schemas" / "init.sql"
    if not schema_path.exists():
        logger.error("Schema file not found: %s", schema_path)
        return

    con = get_db()
    schema_sql = schema_path.read_text()
    # Remove SQL comments and execute as a single script
    lines = []
    for line in schema_sql.splitlines():
        stripped = line.strip()
        if stripped.startswith("--"):
            continue
        lines.append(line)
    clean_sql = "\n".join(lines)
    try:
        con.execute(clean_sql)
    except Exception as e:
        logger.warning("Schema init warning (may be OK if tables exist): %s", e)
    con.close()
    logger.info("DuckDB initialized at %s", db_path())


# ─── Feed Item Storage ───

def store_feed(feed: FeedSourceConfig) -> int:
    con = get_db()
    try:
        existing = con.execute(
            "SELECT id FROM feeds WHERE name = ?", (feed.name,)
        ).fetchone()
        if existing:
            feed_id = existing[0]
            con.execute(
                """UPDATE feeds SET source_type=?, url=?, driver=?, channel_id=?,
                   variant=?, config_json=?, trust=?, enabled=?, check_interval=?
                   WHERE id=?""",
                (feed.source_type, feed.url, feed.driver, feed.channel_id,
                 feed.variant, json.dumps(feed.extra), feed.trust, feed.enabled,
                 feed.interval, feed_id),
            )
        else:
            result = con.execute(
                """INSERT INTO feeds (name, source_type, url, driver, channel_id,
                   variant, config_json, trust, enabled, check_interval)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   RETURNING id""",
                (feed.name, feed.source_type, feed.url, feed.driver, feed.channel_id,
                 feed.variant, json.dumps(feed.extra), feed.trust, feed.enabled, feed.interval),
            )
            feed_id = result.fetchone()[0]
        return feed_id
    finally:
        con.close()


def store_item(feed_id: int, item: dict) -> int | None:
    con = get_db()
    try:
        fp = content_fingerprint(item.get("title", ""), item.get("url", ""))
        existing = con.execute(
            "SELECT 1 FROM seen WHERE feed_id=? AND dedup_hash=?",
            (feed_id, fp),
        ).fetchone()
        if existing:
            return None

        title = sanitize_html(normalize_unicode(item.get("title", "")), strip_all=True)
        summary = sanitize_html(normalize_unicode(item.get("summary", "")), strip_all=True)
        content_text = sanitize_html(normalize_unicode(item.get("content", "")), strip_all=True)
        tags = [normalize_unicode(t.lower().strip())[:50] for t in item.get("tags", [])[:20]]
        word_count = len(content_text.split()) if content_text else 0

        ok, reason = validate_url(item.get("url", "https://example.com"))
        if not ok:
            log_security_event("invalid_item_url", url=item.get("url", ""), reason=reason)
            return None

        content_hash = content_fingerprint(title, item.get("url", ""))
        published_at = item.get("published_at") or datetime.now(timezone.utc).isoformat()

        result = con.execute(
            """INSERT INTO items (feed_id, external_id, title, url, author, summary,
               content, published_at, tags, content_hash, word_count, source_trust)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               RETURNING id""",
            (feed_id, item.get("external_id", ""), title, item.get("url", ""),
             item.get("author", ""), summary, content_text, published_at,
             tags, content_hash, word_count, item.get("trust", "medium")),
        )
        item_id = result.fetchone()[0]

        con.execute(
            "INSERT INTO seen (feed_id, dedup_hash) VALUES (?, ?)",
            (feed_id, fp),
        )

        _index_content(con, item_id, content_text)
        return item_id
    finally:
        con.close()


def _index_content(con, item_id: int, content: str) -> None:
    if not content:
        return
    words = content.split()
    chunk_size = 200
    for i in range(0, len(words), chunk_size):
        chunk_words = words[i:i + chunk_size]
        chunk_text = " ".join(chunk_words)
        con.execute(
            "INSERT INTO search_index (item_id, chunk_index, chunk_text, word_offsets) VALUES (?, ?, ?, ?)",
            (item_id, i // chunk_size, chunk_text, json.dumps({"start": i, "end": i + len(chunk_words)})),
        )


def prune_seen(days: int = 90) -> int:
    con = get_db()
    try:
        result = con.execute(
            "DELETE FROM seen WHERE first_seen < NOW() - INTERVAL ? DAY",
            (days,),
        )
        count = result.rowcount if hasattr(result, 'rowcount') else 0
        return count
    finally:
        con.close()


def get_feed_id_by_name(name: str) -> int | None:
    con = get_db(read_only=True)
    try:
        row = con.execute("SELECT id FROM feeds WHERE name = ?", (name,)).fetchone()
        return row[0] if row else None
    finally:
        con.close()


def get_all_feeds() -> list[dict]:
    con = get_db(read_only=True)
    try:
        rows = con.execute("SELECT id, name, source_type, url, trust, enabled, check_interval FROM feeds").fetchall()
        return [{"id": r[0], "name": r[1], "source_type": r[2], "url": r[3],
                 "trust": r[4], "enabled": r[5], "check_interval": r[6]} for r in rows]
    finally:
        con.close()


def get_item(item_id: int) -> dict | None:
    con = get_db(read_only=True)
    try:
        row = con.execute(
            "SELECT id, feed_id, external_id, title, url, author, summary, content, "
            "published_at, ingested_at, tags, source_trust, word_count FROM items WHERE id = ?",
            (item_id,),
        ).fetchone()
        if not row:
            return None
        return {
            "id": row[0], "feed_id": row[1], "external_id": row[2], "title": row[3],
            "url": row[4], "author": row[5], "summary": row[6], "content": row[7],
            "published_at": str(row[8]) if row[8] else None,
            "ingested_at": str(row[9]) if row[9] else None,
            "tags": row[10] or [], "source_trust": row[11], "word_count": row[12],
        }
    finally:
        con.close()


def get_items(limit: int = 50, source: str = "", tags: list[str] | None = None) -> list[dict]:
    con = get_db(read_only=True)
    try:
        conditions = []
        params: list[Any] = []
        if source:
            conditions.append("f.name = ?")
            params.append(source)
        if tags:
            conditions.append("i.tags && ?")
            params.append(tags)

        where = ""
        if conditions:
            where = "WHERE " + " AND ".join(conditions)

        query = f"""
            SELECT i.id, i.feed_id, i.title, i.url, i.author, i.summary,
                   i.published_at, i.ingested_at, i.tags, i.source_trust, i.word_count,
                   f.name as source_name, f.source_type
            FROM items i
            LEFT JOIN feeds f ON i.feed_id = f.id
            {where}
            ORDER BY i.published_at DESC NULLS LAST
            LIMIT ?
        """
        params.append(limit)
        rows = con.execute(query, params).fetchall()
        return [{"id": r[0], "feed_id": r[1], "title": r[2], "url": r[3],
                 "author": r[4], "summary": r[5],
                 "published_at": str(r[6]) if r[6] else None,
                 "ingested_at": str(r[7]) if r[7] else None,
                 "tags": r[8] or [], "source_trust": r[9], "word_count": r[10],
                 "source_name": r[11], "source_type": r[12]} for r in rows]
    finally:
        con.close()


def get_stats() -> dict:
    con = get_db(read_only=True)
    try:
        total_items = con.execute("SELECT COUNT(*) FROM items").fetchone()[0]
        total_feeds = con.execute("SELECT COUNT(*) FROM feeds").fetchone()[0]
        active_feeds = con.execute("SELECT COUNT(*) FROM feeds WHERE enabled = true").fetchone()[0]
        total_alerts = con.execute("SELECT COUNT(*) FROM alerts").fetchone()[0]
        last_ingest = con.execute("SELECT MAX(ingested_at) FROM items").fetchone()[0]
        items_today = con.execute(
            "SELECT COUNT(*) FROM items WHERE ingested_at >= CURRENT_DATE"
        ).fetchone()[0]

        source_stats = con.execute("""
            SELECT f.name, f.source_type, COUNT(i.id) as item_count,
                   MAX(i.ingested_at) as last_item
            FROM feeds f
            LEFT JOIN items i ON f.id = i.feed_id
            GROUP BY f.id, f.name, f.source_type
            ORDER BY item_count DESC
        """).fetchall()

        return {
            "total_items": total_items,
            "total_feeds": total_feeds,
            "active_feeds": active_feeds,
            "total_alerts": total_alerts,
            "last_ingest": str(last_ingest) if last_ingest else None,
            "items_today": items_today,
            "sources": [
                {"name": r[0], "type": r[1], "item_count": r[2],
                 "last_item": str(r[3]) if r[3] else None}
                for r in source_stats
            ],
        }
    finally:
        con.close()


# ─── Alert Helpers ───

def store_alert(alert: AlertConfig) -> int:
    con = get_db()
    try:
        result = con.execute(
            """INSERT INTO alerts (name, match_config, action, deliver_to, hook_command,
               cooldown_minutes, enabled) VALUES (?, ?, ?, ?, ?, ?, ?)
               RETURNING id""",
            (alert.name, json.dumps({
                "keywords": alert.keywords, "tags": alert.tags, "sources": alert.sources,
            }), alert.action, alert.deliver_to, alert.hook_command,
             alert.cooldown_minutes, alert.enabled),
        )
        alert_id = result.fetchone()[0]
        return alert_id
    finally:
        con.close()


def get_alerts() -> list[dict]:
    con = get_db(read_only=True)
    try:
        rows = con.execute("SELECT id, name, match_config, action, deliver_to, hook_command, cooldown_minutes, enabled FROM alerts").fetchall()
        result = []
        for r in rows:
            mc = json.loads(r[2]) if r[2] else {}
            result.append({
                "id": r[0], "name": r[1], "keywords": mc.get("keywords", []),
                "tags": mc.get("tags", []), "sources": mc.get("sources", []),
                "action": r[3], "deliver_to": r[4], "hook_command": r[5],
                "cooldown_minutes": r[6], "enabled": r[7],
            })
        return result
    finally:
        con.close()


def check_alert_cooldown(alert_id: int, cooldown_minutes: int) -> bool:
    con = get_db(read_only=True)
    try:
        row = con.execute(
            """SELECT delivered_at FROM alert_log
               WHERE alert_id = ? AND success = true
               ORDER BY delivered_at DESC LIMIT 1""",
            (alert_id,),
        ).fetchone()
        if not row:
            return True
        last = row[0]
        if isinstance(last, str):
            from datetime import datetime as dt
            last = dt.fromisoformat(last)
        elapsed = (datetime.now(timezone.utc) - last.replace(tzinfo=timezone.utc)).total_seconds() / 60
        return elapsed >= cooldown_minutes
    finally:
        con.close()


def log_alert(alert_id: int, item_id: int, match_score: float, match_reasons: list,
              action: str, success: bool, detail: str = "") -> None:
    con = get_db()
    try:
        con.execute(
            """INSERT INTO alert_log (alert_id, item_id, match_score, match_reasons,
               action, success, detail) VALUES (?, ?, ?, ?, ?, ?, ?)""",
            (alert_id, item_id, match_score, json.dumps(match_reasons), action, success, detail),
        )
    finally:
        con.close()


# ─── Initialization ───

def init_feed_system(workspace: str | Path | None = None) -> FeedConfig:
    init_security_log(get_data_home())
    init_db()
    config = load_feed_config(workspace)
    logger.info("Feed system initialized: %d sources, %d alerts", len(config.sources), len(config.alerts))
    return config
