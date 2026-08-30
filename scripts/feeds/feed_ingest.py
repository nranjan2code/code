"""
feed_ingest.py — Main ingestion script.

Runs via cron task. Fetches all enabled feeds, deduplicates, stores in DuckDB,
evaluates alert rules, and fires hooks/delivers alerts.

Usage:
    python3 feed_ingest.py --workspace /path/to/project
    python3 feed_ingest.py --workspace /path/to/project --source "TechCrunch"
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from feed_security import detect_injection, log_security_event, sanitize_for_llm
from feed_utils import (
    AlertConfig,
    FeedConfig,
    FeedSourceConfig,
    get_db,
    get_feed_id_by_name,
    init_feed_system,
    load_feed_config,
    log_alert,
    parse_interval,
    prune_seen,
    remove_feed_row,
    store_feed,
    store_item,
    update_feed_row,
)
from sources import get_driver

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(name)s] %(levelname)s: %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S",
)
logger = logging.getLogger("feed_ingest")


def ingest_source(source: FeedSourceConfig, max_items: int = 50) -> int:
    """Fetch and store items from a single source. Returns count of new items."""
    logger.info("Ingesting source: %s (type=%s)", source.name, source.source_type)

    try:
        source_type = source.source_type
        driver_map = {
            "rss": ("rss", None),
            "youtube": ("youtube", None),
            "hacker_news": ("aggregator", "hacker_news"),
            "reddit": ("aggregator", "reddit"),
            "lobsters": ("aggregator", "lobsters"),
            "custom_http": ("custom", None),
        }
        driver_name, variant = driver_map.get(source_type, (source_type, None))
        driver = get_driver(driver_name)
    except ValueError as e:
        logger.error("Driver error for %s: %s", source.name, e)
        return 0

    fetch_kwargs = {}
    if source.source_type == "aggregator":
        fetch_kwargs["driver"] = source.driver
        fetch_kwargs["variant"] = source.variant
        fetch_kwargs["subreddit"] = source.extra.get("subreddit", "")
        fetch_kwargs["sort"] = source.sort
    elif source.source_type == "custom":
        fetch_kwargs["format"] = source.format
        fetch_kwargs["json_path"] = source.json_path
        fetch_kwargs["mapping"] = source.mapping

    try:
        fetch_args = {}
        if variant is not None:
            fetch_args["driver"] = variant
        items = driver.fetch(
            url=source.url,
            tags=source.tags,
            trust=source.trust,
            max_items=max_items,
            **fetch_args,
            **fetch_kwargs,
        )
    except Exception as e:
        logger.error("Fetch error for %s: %s", source.name, e)
        return 0

    if not items:
        logger.info("No items from %s", source.name)
        return 0

    feed_id = get_feed_id_by_name(source.name)
    if feed_id is None:
        feed_id = store_feed(source)

    new_count = 0
    for item in items:
        item_id = store_item(feed_id, item)
        if item_id is not None:
            new_count += 1

    logger.info("Stored %d new items from %s (of %d fetched)", new_count, source.name, len(items))
    return new_count


def evaluate_alerts(config: FeedConfig) -> list[dict]:
    """Evaluate alert rules against recent items. Returns list of fired alerts."""
    con = get_db()
    fired = []

    for alert in config.alerts:
        if not alert.enabled:
            continue

        # Look up the real alert ID from the database
        alert_row = con.execute(
            "SELECT id FROM alerts WHERE name = ?", (alert.name,)
        ).fetchone()
        alert_id = alert_row[0] if alert_row else 0

        conditions = []
        params: list = []

        if alert.keywords:
            keyword_conditions = []
            for kw in alert.keywords:
                keyword_conditions.append("(i.title ILIKE ? OR i.summary ILIKE ? OR i.content ILIKE ?)")
                pattern = f"%{kw}%"
                params.extend([pattern, pattern, pattern])
            conditions.append(f"({' OR '.join(keyword_conditions)})")

        if alert.tags:
            conditions.append("i.tags && ?")
            params.append(alert.tags)

        if alert.sources:
            conditions.append("f.name IN ({})".format(
                ", ".join(["?" for _ in alert.sources])
            ))
            params.extend(alert.sources)

        if not conditions:
            continue

        where = " AND ".join(conditions)
        query = f"""
            SELECT i.id, i.title, i.url, i.summary, i.tags, i.source_trust,
                   f.name as source_name, f.source_type, i.content
            FROM items i
            LEFT JOIN feeds f ON i.feed_id = f.id
            WHERE {where}
            ORDER BY i.ingested_at DESC
            LIMIT 10
        """

        try:
            rows = con.execute(query, params).fetchall()
        except Exception as e:
            logger.error("Alert query error for %s: %s", alert.name, e)
            continue

        for row in rows:
            item_id = row[0]
            if not check_alert_cooldown_by_name(alert):
                continue

            match_score = _compute_match_score(alert, row)
            match_reasons = _compute_match_reasons(alert, row)

            log_alert(
                alert_id=alert_id, item_id=item_id, match_score=match_score,
                match_reasons=match_reasons, action=alert.action, success=True,
            )

            fired.append({
                "alert_id": alert_id,
                "alert_name": alert.name,
                "item": {
                    "id": row[0], "title": row[1], "url": row[2],
                    "summary": row[3], "tags": row[4], "source_trust": row[5],
                    "source_name": row[6], "source_type": row[7],
                },
                "match": {"score": match_score, "reasons": match_reasons},
                "action": alert.action,
                "deliver_to": alert.deliver_to,
                "hook_command": alert.hook_command,
            })

    con.close()
    return fired


def check_alert_cooldown_by_name(alert: AlertConfig) -> bool:
    con = get_db()
    row = con.execute(
        """SELECT al.delivered_at FROM alert_log al
           JOIN alerts a ON al.alert_id = a.id
           WHERE a.name = ? AND al.success = true
           ORDER BY al.delivered_at DESC LIMIT 1""",
        (alert.name,),
    ).fetchone()
    con.close()
    if not row:
        return True
    last = row[0]
    if isinstance(last, str):
        last = datetime.fromisoformat(last)
    if last.tzinfo is None:
        last = last.replace(tzinfo=timezone.utc)
    elapsed = (datetime.now(timezone.utc) - last).total_seconds() / 60
    return elapsed >= alert.cooldown_minutes


def _compute_match_score(alert: AlertConfig, row: tuple) -> float:
    score = 0.0
    title = (row[1] or "").lower()
    summary = (row[3] or "").lower()
    content = (row[8] or "").lower()
    tags = row[4] or []
    trust = row[5] or "medium"

    for kw in alert.keywords:
        kw_lower = kw.lower()
        if kw_lower in title:
            score += 0.4
        elif kw_lower in summary:
            score += 0.3
        elif kw_lower in content:
            score += 0.2

    if alert.tags:
        tag_overlap = len(set(alert.tags) & set(tags))
        score += 0.2 * min(tag_overlap / len(alert.tags), 1.0)

    trust_bonus = {"high": 0.1, "medium": 0.05, "low": 0.0}.get(trust, 0.0)
    score += trust_bonus

    return min(score, 1.0)


def _compute_match_reasons(alert: AlertConfig, row: tuple) -> list[str]:
    reasons = []
    title = (row[1] or "").lower()
    summary = (row[3] or "").lower()
    content = (row[8] or "").lower()
    tags = row[4] or []
    trust = row[5] or "medium"

    for kw in alert.keywords:
        kw_lower = kw.lower()
        if kw_lower in title:
            reasons.append(f"keyword:{kw} (title)")
        elif kw_lower in summary:
            reasons.append(f"keyword:{kw} (summary)")
        elif kw_lower in content:
            reasons.append(f"keyword:{kw} (content)")

    for tag in alert.tags:
        if tag in tags:
            reasons.append(f"tag:{tag}")

    reasons.append(f"source_trust:{trust}")
    return reasons


def fire_alerts(fired_alerts: list[dict]) -> None:
    """Deliver alerts and fire hooks."""
    for alert in fired_alerts:
        payload = json.dumps({
            "event": "feed_item_matched",
            "alert": {
                "id": alert.get("alert_id", 0),
                "name": alert["alert_name"],
                "action": alert["action"],
            },
            "item": alert["item"],
            "match": alert["match"],
        }, indent=2)

        if alert["action"] in ("deliver", "both") and alert["deliver_to"]:
            deliver_to(alert["deliver_to"], alert)

        if alert["action"] in ("hook", "both") and alert["hook_command"]:
            fire_hook(alert["hook_command"], payload)


def deliver_to(target: str, alert: dict) -> None:
    """Deliver alert via gateway delivery outbox.

    Writes an outbox record to <sessions_home>/delivery/jobs/ so the gateway
    delivery runtime picks it up and sends to the configured surface.
    """
    logger.info("Delivering alert '%s' to %s", alert["alert_name"], target)
    item = alert["item"]
    message = (
        f"**Feed Alert: {alert['alert_name']}**\n\n"
        f"**{item.get('title', '')}**\n"
        f"Source: {item.get('source_name', '')}\n"
        f"URL: {item.get('url', '')}\n"
        f"Match score: {alert['match']['score']:.2f}\n"
        f"Reasons: {', '.join(alert['match']['reasons'])}"
    )

    # Determine sessions_home from environment or default
    data_home = Path(os.environ.get("VAK_DATA_HOME", "")) or Path.home() / ".local" / "share" / "vak"
    if os.name == "darwin":
        data_home = Path(os.environ.get("XDG_DATA_HOME", "")) or Path.home() / "Library" / "Application Support" / "vak"
    jobs_dir = data_home / "delivery" / "jobs"
    jobs_dir.mkdir(parents=True, exist_ok=True)

    # Generate a unique job ID
    import hashlib
    job_id = hashlib.sha256(
        f"{alert['alert_name']}:{item.get('url', '')}:{datetime.now(timezone.utc).isoformat()}".encode()
    ).hexdigest()[:32]

    # Build the outbox record matching vak-delivery's OutboxRecord schema
    now_ms = int(datetime.now(timezone.utc).timestamp() * 1000)
    record = {
        "schema_version": 1,
        "job": {
            "job_id": job_id,
            "target": target,
            "kind": "alert",
            "content": {
                "type": "text",
                "markdown": message,
            },
            "profile": {
                "surface": target,
                "markup": "markdown",
                "max_chars": None,
                "supports_tables": False,
                "supports_code_blocks": True,
                "supports_links": True,
                "supports_actions": False,
            },
        },
        "state": "pending",
        "attempts": 0,
        "created_at_ms": now_ms,
        "updated_at_ms": now_ms,
        "packet": None,
        "last_error": None,
    }

    record_path = jobs_dir / f"{job_id}.json"
    try:
        with open(record_path, "w") as f:
            json.dump(record, f)
        logger.info("Alert '%s' enqueued to outbox: %s", alert["alert_name"], record_path)
    except Exception as e:
        logger.error("Failed to write outbox record for alert '%s': %s", alert["alert_name"], e)

    # Also write to local log for audit trail
    log_dir = data_home / "feeds" / "alerts"
    log_dir.mkdir(parents=True, exist_ok=True)
    log_file = log_dir / f"{datetime.now().strftime('%Y-%m-%d')}.log"

    with open(log_file, "a") as f:
        f.write(json.dumps({
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "target": target,
            "alert_name": alert["alert_name"],
            "item_title": item.get("title", ""),
            "item_url": item.get("url", ""),
            "match_score": alert["match"]["score"],
            "outbox_job_id": job_id,
        }) + "\n")


def fire_hook(command: str, payload: str) -> None:
    """Execute a hook script with JSON payload on stdin."""
    logger.info("Firing hook: %s", command)
    try:
        proc = subprocess.Popen(
            ["sh", "-c", command],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        stdout, stderr = proc.communicate(input=payload.encode(), timeout=30)
        if proc.returncode != 0:
            logger.error("Hook failed (exit %d): %s", proc.returncode, stderr.decode()[:500])
        else:
            logger.info("Hook succeeded: %s", stdout.decode()[:200])
    except subprocess.TimeoutExpired:
        proc.kill()
        logger.error("Hook timed out: %s", command)
    except Exception as e:
        logger.error("Hook error: %s — %s", command, e)


def run_ingestion(workspace: str | None = None, source_name: str | None = None) -> dict:
    """Main ingestion entry point. Returns summary stats."""
    config = init_feed_system(workspace)

    start_time = time.time()
    total_new = 0
    sources_ingested = 0
    errors = 0

    for source in config.sources:
        if source_name and source.name != source_name:
            continue

        try:
            count = ingest_source(source)
            total_new += count
            sources_ingested += 1
        except Exception as e:
            logger.error("Error ingesting %s: %s", source.name, e)
            errors += 1

    fired = []
    try:
        fired = evaluate_alerts(config)
        fire_alerts(fired)
    except Exception as e:
        logger.error("Error evaluating alerts: %s", e)
        errors += 1

    prune_seen(config.dedup_window_days)

    elapsed = time.time() - start_time
    summary = {
        "sources_ingested": sources_ingested,
        "new_items": total_new,
        "alerts_fired": len(fired),
        "errors": errors,
        "elapsed_seconds": round(elapsed, 2),
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }
    logger.info("Ingestion complete: %s", json.dumps(summary))
    return summary


def main():
    parser = argparse.ArgumentParser(description="Feed ingestion pipeline")
    parser.add_argument("--workspace", "-w", help="Workspace path")
    parser.add_argument("--source", "-s", help="Ingest a specific source by name")
    parser.add_argument("--init", action="store_true", help="Initialize DB only")
    parser.add_argument("--stats", action="store_true", help="Print stats only")
    # Admin-console CRUD against the live `feeds` table -- see
    # feed_utils.update_feed_row/remove_feed_row for why this operates on
    # the DB directly instead of feeds.toml (which is what the UI
    # actually displays and what update/remove in
    # crates/vak-server/src/feeds.rs call through to). Not exposed via
    # feed_mcp.py's tool list: that surface is reachable by LLM agents,
    # and source removal shouldn't be an agent-callable tool.
    parser.add_argument("--remove-source", metavar="NAME", help="Soft-delete a source by name")
    parser.add_argument("--update-source", metavar="NAME", help="Update a source by name")
    parser.add_argument("--set-enabled", choices=["true", "false"], help="With --update-source")
    parser.add_argument("--set-interval", help="With --update-source")
    parser.add_argument("--set-trust", choices=["high", "medium", "low"], help="With --update-source")
    args = parser.parse_args()

    if args.init:
        init_feed_system(args.workspace)
        print("Feed system initialized.")
        return

    if args.stats:
        from feed_utils import get_stats
        init_feed_system(args.workspace)
        stats = get_stats()
        print(json.dumps(stats, indent=2))
        return

    if args.remove_source:
        init_feed_system(args.workspace)
        ok = remove_feed_row(args.remove_source)
        print(json.dumps({"status": "ok" if ok else "not_found", "name": args.remove_source}))
        sys.exit(0 if ok else 1)

    if args.update_source:
        init_feed_system(args.workspace)
        enabled = None
        if args.set_enabled is not None:
            enabled = args.set_enabled == "true"
        ok = update_feed_row(
            args.update_source,
            enabled=enabled,
            interval=args.set_interval,
            trust=args.set_trust,
        )
        print(json.dumps({"status": "ok" if ok else "not_found", "name": args.update_source}))
        sys.exit(0 if ok else 1)

    summary = run_ingestion(args.workspace, args.source)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
