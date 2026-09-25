"""
feed_utils.py — Config loading, DuckDB helpers, dedup, and shared utilities.
"""

from __future__ import annotations

import json
import logging
import os
import sys
import time
try:
    import tomllib
except ModuleNotFoundError:
    try:
        import tomli as tomllib
    except ModuleNotFoundError:
        import toml as tomllib
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

# ─── Paths ───
#
# The server decides every path this pipeline writes, from the canonical
# data home (which follows VAK_HOME), and passes it in. The pipeline never
# guesses one: a guess ignored VAK_HOME and wrote outside an overridden home.

def _host_path(name: str) -> Path:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"{name} is not set; feed scripts run under vak, which sets it")
    return Path(value)


def db_path() -> Path:
    path = _host_path("VAK_FEEDS_DB")
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def global_config_path() -> Path:
    return _host_path("VAK_FEEDS_CONFIG")


def security_log_path() -> Path:
    return _host_path("VAK_FEEDS_LOG")


def current_workspace_id() -> str:
    """Return the canonical scope key supplied by the host process."""
    return os.environ.get("VAK_FEED_WORKSPACE", "")


def begin_ingestion_run(source_id: str = "") -> int:
    con = get_db()
    try:
        workspace = current_workspace_id()
        scope = "workspace" if workspace else "global"
        row = con.execute(
            """INSERT INTO ingestion_runs (run_key, scope, workspace_id, source_id, status)
               VALUES (?, ?, ?, ?, 'running') RETURNING id""",
            (f"{scope}:{workspace}:{source_id}", scope, workspace, source_id or None),
        ).fetchone()
        return int(row[0])
    finally:
        con.close()


def finish_ingestion_run(run_id: int, status: str, *, sources_seen: int = 0,
                         sources_succeeded: int = 0, items_seen: int = 0,
                         items_added: int = 0, error: str = "") -> None:
    con = get_db()
    try:
        con.execute(
            """UPDATE ingestion_runs SET status=?, finished_at=current_timestamp,
               sources_seen=?, sources_succeeded=?, items_seen=?, items_added=?, error=?
               WHERE id=?""",
            (status, sources_seen, sources_succeeded, items_seen, items_added, error or None, run_id),
        )
    finally:
        con.close()


def source_is_due(source_id: str, scope: str, workspace_id: str) -> bool:
    con = get_db(read_only=True)
    try:
        row = con.execute(
            """SELECT next_due_at FROM feeds
               WHERE source_id = ? AND scope = ? AND workspace_id = ?
                 AND removed_at IS NULL AND enabled = true""",
            (source_id, scope, workspace_id),
        ).fetchone()
        return row is None or row[0] is None or row[0] <= datetime.now(timezone.utc).replace(tzinfo=None)
    finally:
        con.close()


def mark_source_checked(source: FeedSourceConfig) -> None:
    con = get_db()
    try:
        seconds = parse_interval(source.interval)
        con.execute(
            """UPDATE feeds
               SET last_started_at = current_timestamp,
                   next_due_at = current_timestamp + (? * INTERVAL '1 second')
               WHERE source_id = ? AND scope = ? AND workspace_id = ?""",
            (seconds, source.id, source.scope, source.workspace_id),
        )
    finally:
        con.close()


def record_source_outcome(source: FeedSourceConfig, status: str, error: str = "") -> None:
    con = get_db()
    try:
        con.execute(
            """UPDATE feeds SET last_status=?, last_error=?
               WHERE source_id=? AND scope=? AND workspace_id=?""",
            (status, error or None, source.id, source.scope, source.workspace_id),
        )
    finally:
        con.close()


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
    scope: str = "global"
    workspace_id: str = ""


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
    scope: str = "global"
    workspace_id: str = ""


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
    if not interval:
        raise ValueError("feed interval cannot be empty")
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
    if workspace is None:
        workspace = os.environ.get("VAK_FEED_WORKSPACE") or None
    global_data = load_toml(global_config_path())

    workspace_config_path = None
    if workspace:
        ws = Path(workspace)
        workspace_config_path = ws / ".vak" / "feeds.toml"
    workspace_data = load_toml(workspace_config_path) if workspace_config_path else {}

    general = {**global_data.get("general", {}), **workspace_data.get("general", {})}
    sources_raw = [
        {**s, "scope": "global", "workspace_id": ""}
        for s in global_data.get("sources", [])
    ] + [
        {**s, "scope": "workspace", "workspace_id": str(workspace or "")}
        for s in workspace_data.get("sources", [])
    ]
    alerts_raw = [
        {**a, "scope": "global", "workspace_id": ""}
        for a in global_data.get("alerts", [])
    ] + [
        {**a, "scope": "workspace", "workspace_id": str(workspace or "")}
        for a in workspace_data.get("alerts", [])
    ]

    sources = []
    seen_ids: set[str] = set()
    for s in sources_raw:
        name = s.get("name", "")
        source_id = s.get("id", name.lower().replace(" ", "-"))
        if source_id in seen_ids:
            # The narrower workspace layer overrides the inherited source.
            if s.get("scope") != "workspace":
                continue
            sources = [source for source in sources if source.id != source_id]
        seen_ids.add(source_id)
        if not s.get("enabled", True):
            continue
        if not name:
            continue
        sources.append(FeedSourceConfig(
            id=source_id,
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
            scope=s.get("scope", "global"),
            workspace_id=s.get("workspace_id", ""),
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
            scope=a.get("scope", "global"),
            workspace_id=a.get("workspace_id", ""),
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
        # Checks removed_at specifically, not just that `feeds` exists --
        # that column was added after the table itself, and a DB created
        # before that migration must still take the init_db() write path
        # once so ALTER TABLE ... ADD COLUMN IF NOT EXISTS actually runs.
        con.execute("SELECT removed_at FROM feeds LIMIT 0")
        con.execute("SELECT security_detail FROM items LIMIT 0")
        con.execute("SELECT scope, workspace_id FROM alerts LIMIT 0")
        con.execute("SELECT items_quarantined FROM ingestion_runs LIMIT 0")
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
            "SELECT id FROM feeds WHERE source_id = ? AND scope = ? AND workspace_id = ?",
            (feed.id, feed.scope, feed.workspace_id),
        ).fetchone()
        if existing:
            feed_id = existing[0]
            con.execute(
                """UPDATE feeds SET name=?, source_type=?, url=?, driver=?, channel_id=?,
                   variant=?, config_json=?, trust=?, enabled=?, check_interval=?,
                   removed_at=NULL, security_status='accepted'
                   WHERE id=?""",
                (feed.name, feed.source_type, feed.url, feed.driver, feed.channel_id,
                 feed.variant, json.dumps(feed.extra), feed.trust, feed.enabled,
                 feed.interval, feed_id),
            )
        else:
            result = con.execute(
                """INSERT INTO feeds (name, source_type, url, driver, channel_id,
                   variant, config_json, trust, enabled, check_interval, source_id,
                   scope, workspace_id, security_status)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   RETURNING id""",
                (feed.name, feed.source_type, feed.url, feed.driver, feed.channel_id,
                 feed.variant, json.dumps(feed.extra), feed.trust, feed.enabled,
                 feed.interval, feed.id, feed.scope, feed.workspace_id, "accepted"),
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

        injection_fields = []
        for field, value in (("title", title), ("summary", summary), ("content", content_text)):
            detected, _ = detect_injection(value)
            if detected:
                injection_fields.append(field)
        security_status = "quarantined" if injection_fields else "accepted"
        security_detail = (
            "prompt-injection indicators in " + ", ".join(injection_fields)
            if injection_fields else None
        )
        if injection_fields:
            log_security_event(
                "feed_item_quarantined", url=item.get("url", ""),
                reason=security_detail or "prompt-injection indicators",
            )

        ok, reason = validate_url(item.get("url", "https://example.com"))
        if not ok:
            log_security_event("invalid_item_url", url=item.get("url", ""), reason=reason)
            return None

        content_hash = content_fingerprint(title, item.get("url", ""))
        published_at = item.get("published_at") or datetime.now(timezone.utc).isoformat()
        feed_scope = con.execute(
            "SELECT scope, workspace_id FROM feeds WHERE id = ?",
            (feed_id,),
        ).fetchone()
        if not feed_scope:
            return None

        result = con.execute(
            """INSERT INTO items (feed_id, external_id, title, url, author, summary,
               content, published_at, tags, content_hash, word_count, source_trust,
               security_status, security_detail, scope, workspace_id)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               RETURNING id""",
            (feed_id, item.get("external_id", ""), title, item.get("url", ""),
             item.get("author", ""), summary, content_text, published_at,
             tags, content_hash, word_count, item.get("trust", "medium"),
             security_status, security_detail, feed_scope[0], feed_scope[1]),
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
    # DuckDB doesn't accept a placeholder for the numeric part of an
    # INTERVAL literal ("INTERVAL ? DAY" is a parser error, not a runtime
    # one -- it fails on every call). `? * INTERVAL 1 DAY` parameterizes
    # a multiplication instead, which DuckDB does support, and keeps
    # `days` as a bound parameter rather than a string-interpolated value.
    con = get_db()
    try:
        result = con.execute(
            "DELETE FROM seen WHERE first_seen < NOW() - (? * INTERVAL 1 DAY)",
            (int(days),),
        )
        count = result.rowcount if hasattr(result, 'rowcount') else 0
        return count
    finally:
        con.close()


def get_feed_id_by_name(name: str) -> int | None:
    """Resolve a source row by its stable ID (legacy function name)."""
    con = get_db(read_only=True)
    try:
        row = con.execute(
            """SELECT id FROM feeds
               WHERE source_id = ? AND removed_at IS NULL
                 AND (scope = 'global' OR workspace_id = ?)
               ORDER BY CASE WHEN workspace_id = ? THEN 0 ELSE 1 END
               LIMIT 1""",
            (name, current_workspace_id(), current_workspace_id()),
        ).fetchone()
        return row[0] if row else None
    finally:
        con.close()


def get_all_feeds() -> list[dict]:
    con = get_db(read_only=True)
    try:
        rows = con.execute(
            "SELECT id, name, source_type, url, trust, enabled, check_interval "
            "FROM feeds WHERE removed_at IS NULL "
            "AND (scope = 'global' OR workspace_id = ?)",
            (current_workspace_id(),),
        ).fetchall()
        return [{"id": r[0], "name": r[1], "source_type": r[2], "url": r[3],
                 "trust": r[4], "enabled": r[5], "check_interval": r[6]} for r in rows]
    finally:
        con.close()


# ─── Source CRUD (admin console) ───
#
# The materialized `feeds` table is the read model used by the admin UI and
# ingestion. Configuration remains the durable declaration for the selected
# scope; mutations update the materialized row and reconcile that declaration.
# Rows are never hard-deleted: items retain their source attribution and
# removal is represented by `removed_at`.

def update_feed_row(
    source_id: str,
    enabled: bool | None = None,
    interval: str | None = None,
    trust: str | None = None,
    scope: str | None = None,
) -> bool:
    """Update a feeds row by stable source id."""
    sets: list[str] = []
    params: list[Any] = []
    if enabled is not None:
        sets.append("enabled = ?")
        params.append(enabled)
    if interval is not None:
        sets.append("check_interval = ?")
        params.append(interval)
    if trust is not None:
        sets.append("trust = ?")
        params.append(trust)
    if not sets:
        return False

    con = get_db()
    try:
        # DuckDB's cursor.rowcount is always -1 for UPDATE (unreliable for
        # existence checks, and -1 is truthy in Python besides), so check
        # separately rather than trust it.
        scope = scope or ("workspace" if current_workspace_id() else "global")
        workspace_id = current_workspace_id() if scope == "workspace" else ""
        exists = con.execute(
            """SELECT 1 FROM feeds WHERE source_id = ? AND scope = ? AND workspace_id = ?
               AND removed_at IS NULL""", (source_id, scope, workspace_id),
        ).fetchone()
        if not exists:
            return False
        params.append(source_id)
        con.execute(
            f"UPDATE feeds SET {', '.join(sets)} "
            "WHERE source_id = ? AND scope = ? AND workspace_id = ? AND removed_at IS NULL",
            [*params, source_id, scope, workspace_id],
        )
        return True
    finally:
        con.close()


def remove_feed_row(source_id: str, scope: str | None = None) -> bool:
    """Soft-delete a feeds row by stable source id."""
    con = get_db()
    try:
        scope = scope or ("workspace" if current_workspace_id() else "global")
        workspace_id = current_workspace_id() if scope == "workspace" else ""
        exists = con.execute(
            """SELECT 1 FROM feeds WHERE source_id = ? AND scope = ? AND workspace_id = ?
               AND removed_at IS NULL""", (source_id, scope, workspace_id),
        ).fetchone()
        if not exists:
            return False
        con.execute(
            "UPDATE feeds SET enabled = false, removed_at = current_timestamp "
            "WHERE source_id = ? AND scope = ? AND workspace_id = ? AND removed_at IS NULL",
            (source_id, scope, workspace_id),
        )
        return True
    finally:
        con.close()


def get_item(item_id: int) -> dict | None:
    con = get_db(read_only=True)
    try:
        row = con.execute(
            """SELECT i.id, i.feed_id, i.external_id, i.title, i.url, i.author,
                       i.summary, i.content, i.published_at, i.ingested_at,
                       i.tags, i.source_trust, i.word_count, i.scope, i.workspace_id,
                       i.security_status, i.security_detail, f.source_id
                FROM items i JOIN feeds f ON f.id = i.feed_id
                WHERE i.id = ? AND i.security_status = 'accepted'
                  AND f.removed_at IS NULL
                  AND (f.scope = 'global' OR f.workspace_id = ?)""",
            (item_id, current_workspace_id()),
        ).fetchone()
        if not row:
            return None
        return {
            "id": row[0], "feed_id": row[1], "external_id": row[2], "title": row[3],
            "url": row[4], "author": row[5], "summary": row[6], "content": row[7],
            "published_at": str(row[8]) if row[8] else None,
            "ingested_at": str(row[9]) if row[9] else None,
            "tags": row[10] or [], "source_trust": row[11], "word_count": row[12],
            "scope": row[13], "workspace_id": row[14],
            "security_status": row[15], "security_detail": row[16], "source_id": row[17],
        }
    finally:
        con.close()


def get_quarantined_items(limit: int = 50) -> list[dict]:
    con = get_db(read_only=True)
    try:
        rows = con.execute(
            """SELECT i.id, i.title, i.url, i.security_status, i.security_detail,
                      i.ingested_at, f.name, f.scope, f.workspace_id
               FROM items i JOIN feeds f ON f.id = i.feed_id
               WHERE i.security_status = 'quarantined'
                 AND f.removed_at IS NULL
                 AND (f.scope = 'global' OR f.workspace_id = ?)
               ORDER BY i.ingested_at DESC LIMIT ?""",
            (current_workspace_id(), min(max(limit, 1), 100)),
        ).fetchall()
        return [
            {"id": r[0], "title": r[1], "url": r[2], "security_status": r[3],
             "security_detail": r[4], "ingested_at": str(r[5]) if r[5] else None,
             "source_name": r[6], "scope": r[7], "workspace_id": r[8]}
            for r in rows
        ]
    finally:
        con.close()


def set_item_security_status(item_id: int, status: str, detail: str = "") -> bool:
    if status not in {"accepted", "quarantined", "blocked"}:
        raise ValueError(f"invalid security status: {status}")
    con = get_db()
    try:
        result = con.execute(
            """UPDATE items SET security_status = ?, security_detail = ?
               WHERE id = ? AND EXISTS (
                 SELECT 1 FROM feeds f WHERE f.id = items.feed_id
                   AND f.removed_at IS NULL
                   AND (f.scope = 'global' OR f.workspace_id = ?)
               ) RETURNING id""",
            (status, detail or None, item_id, current_workspace_id()),
        ).fetchone()
        return result is not None
    finally:
        con.close()


def mark_alert_delivered(alert_id: int, item_id: int, detail: str = "") -> bool:
    con = get_db()
    try:
        row = con.execute(
            """UPDATE alert_log SET success = true, detail = ?
               WHERE id = (SELECT id FROM alert_log
                           WHERE alert_id = ? AND item_id = ?
                             AND EXISTS (SELECT 1 FROM alerts a
                                         WHERE a.id = alert_log.alert_id
                                           AND (a.scope = 'global' OR a.workspace_id = ?))
                             AND EXISTS (SELECT 1 FROM items i
                                         WHERE i.id = alert_log.item_id
                                           AND (i.scope = 'global' OR i.workspace_id = ?))
                           ORDER BY delivered_at DESC LIMIT 1)
               RETURNING id""",
            (detail or "delivered by host runtime", alert_id, item_id,
             current_workspace_id(), current_workspace_id()),
        ).fetchone()
        return row is not None
    finally:
        con.close()


def get_items(limit: int = 50, source: str = "", tags: list[str] | None = None) -> list[dict]:
    con = get_db(read_only=True)
    try:
        conditions = [
            "f.removed_at IS NULL",
            "i.security_status = 'accepted'",
            "(f.scope = 'global' OR f.workspace_id = ?)",
        ]
        params: list[Any] = [current_workspace_id()]
        if source:
            conditions.append("f.source_id = ?")
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
                   f.name as source_name, f.source_type, i.scope, i.workspace_id,
                   i.security_status, f.source_id
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
                 "source_name": r[11], "source_type": r[12], "scope": r[13],
                 "workspace_id": r[14], "security_status": r[15], "source_id": r[16]} for r in rows]
    finally:
        con.close()


def get_stats() -> dict:
    con = get_db(read_only=True)
    try:
        scope = current_workspace_id()
        total_items = con.execute(
            """SELECT COUNT(*) FROM items i JOIN feeds f ON f.id = i.feed_id
               WHERE f.removed_at IS NULL AND (f.scope = 'global' OR f.workspace_id = ?)""",
            (scope,),
        ).fetchone()[0]
        total_feeds = con.execute(
            "SELECT COUNT(*) FROM feeds WHERE removed_at IS NULL "
            "AND (scope = 'global' OR workspace_id = ?)", (scope,)
        ).fetchone()[0]
        active_feeds = con.execute(
            "SELECT COUNT(*) FROM feeds WHERE enabled = true AND removed_at IS NULL "
            "AND (scope = 'global' OR workspace_id = ?)", (scope,)
        ).fetchone()[0]
        total_alerts = con.execute(
            "SELECT COUNT(*) FROM alerts WHERE scope = 'global' OR workspace_id = ?",
            (scope,),
        ).fetchone()[0]
        last_ingest = con.execute(
            """SELECT MAX(i.ingested_at) FROM items i JOIN feeds f ON f.id = i.feed_id
               WHERE f.removed_at IS NULL AND (f.scope = 'global' OR f.workspace_id = ?)""",
            (scope,),
        ).fetchone()[0]
        items_today = con.execute(
            """SELECT COUNT(*) FROM items i JOIN feeds f ON f.id = i.feed_id
               WHERE i.ingested_at >= CURRENT_DATE AND f.removed_at IS NULL
                 AND (f.scope = 'global' OR f.workspace_id = ?)""",
            (scope,),
        ).fetchone()[0]
        quarantined_items = con.execute(
            """SELECT COUNT(*) FROM items i JOIN feeds f ON f.id = i.feed_id
               WHERE i.security_status = 'quarantined' AND f.removed_at IS NULL
                 AND (f.scope = 'global' OR f.workspace_id = ?)""",
            (scope,),
        ).fetchone()[0]

        source_stats = con.execute("""
            SELECT f.name, f.source_type, COUNT(i.id) as item_count,
                   MAX(i.ingested_at) as last_item
            FROM feeds f
            LEFT JOIN items i ON f.id = i.feed_id
            WHERE f.removed_at IS NULL
              AND (f.scope = 'global' OR f.workspace_id = ?)
            GROUP BY f.id, f.name, f.source_type
            ORDER BY item_count DESC
        """, (scope,)).fetchall()

        return {
            "total_items": total_items,
            "total_feeds": total_feeds,
            "active_feeds": active_feeds,
            "total_alerts": total_alerts,
            "last_ingest": str(last_ingest) if last_ingest else None,
            "items_today": items_today,
            "quarantined_items": quarantined_items,
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
               cooldown_minutes, enabled, scope, workspace_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
               RETURNING id""",
            (alert.name, json.dumps({
                "keywords": alert.keywords, "tags": alert.tags, "sources": alert.sources,
            }), alert.action, alert.deliver_to, alert.hook_command,
             alert.cooldown_minutes, alert.enabled, alert.scope, alert.workspace_id),
        )
        alert_id = result.fetchone()[0]
        return alert_id
    finally:
        con.close()


def sync_alerts(config: FeedConfig) -> int:
    """Materialize configured alerts and disable rules removed from config."""
    con = get_db()
    try:
        active = {(a.name, a.scope, a.workspace_id) for a in config.alerts}
        for alert in config.alerts:
            match_config = json.dumps({"keywords": alert.keywords, "tags": alert.tags, "sources": alert.sources})
            row = con.execute(
                "SELECT id FROM alerts WHERE name=? AND scope=? AND workspace_id=?",
                (alert.name, alert.scope, alert.workspace_id),
            ).fetchone()
            if row:
                con.execute(
                    "UPDATE alerts SET match_config=?, action=?, deliver_to=?, hook_command=?, cooldown_minutes=?, enabled=? WHERE id=?",
                    (match_config, alert.action, alert.deliver_to, alert.hook_command, alert.cooldown_minutes, alert.enabled, row[0]),
                )
            else:
                con.execute(
                    """INSERT INTO alerts (name, match_config, action, deliver_to, hook_command,
                       cooldown_minutes, enabled, scope, workspace_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                    (alert.name, match_config, alert.action, alert.deliver_to, alert.hook_command,
                     alert.cooldown_minutes, alert.enabled, alert.scope, alert.workspace_id),
                )
        rows = con.execute(
            "SELECT id, name, scope, workspace_id FROM alerts WHERE scope='global' OR workspace_id=?",
            (current_workspace_id(),),
        ).fetchall()
        for alert_id, name, scope, workspace_id in rows:
            if (name, scope, workspace_id) not in active:
                con.execute("UPDATE alerts SET enabled=false WHERE id=?", (alert_id,))
        return len(config.alerts)
    finally:
        con.close()


def get_alerts() -> list[dict]:
    con = get_db(read_only=True)
    try:
        workspace = current_workspace_id()
        rows = con.execute(
            """SELECT id, name, match_config, action, deliver_to, hook_command,
                      cooldown_minutes, enabled, scope, workspace_id
               FROM alerts
               WHERE scope = 'global' OR workspace_id = ?""",
            (workspace,),
        ).fetchall()
        result = []
        for r in rows:
            mc = json.loads(r[2]) if r[2] else {}
            result.append({
                "id": r[0], "name": r[1], "keywords": mc.get("keywords", []),
                "tags": mc.get("tags", []), "sources": mc.get("sources", []),
                "action": r[3], "deliver_to": r[4], "hook_command": r[5],
                "cooldown_minutes": r[6], "enabled": r[7],
                "scope": r[8], "workspace_id": r[9],
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
    init_security_log(security_log_path())
    init_db()
    config = load_feed_config(workspace)
    logger.info("Feed system initialized: %d sources, %d alerts", len(config.sources), len(config.alerts))
    return config
