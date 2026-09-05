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
import contextlib
import fcntl
import json
import logging
import os
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
    current_workspace_id,
    begin_ingestion_run,
    finish_ingestion_run,
    mark_source_checked,
    source_is_due,
    record_source_outcome,
    set_item_security_status,
    mark_alert_delivered,
    sync_alerts,
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
        record_source_outcome(source, "failed", str(e))
        return -1

    fetch_kwargs = {}
    if source.source_type == "youtube":
        fetch_kwargs["channel_id"] = source.channel_id or source.url
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
        record_source_outcome(source, "failed", str(e))
        return -1

    if not items:
        logger.info("No items from %s", source.name)
        record_source_outcome(source, "empty")
        return 0

    feed_id = get_feed_id_by_name(source.id)
    if feed_id is None:
        feed_id = store_feed(source)

    new_count = 0
    for item in items:
        item_id = store_item(feed_id, item)
        if item_id is not None:
            new_count += 1

    logger.info("Stored %d new items from %s (of %d fetched)", new_count, source.name, len(items))
    record_source_outcome(source, "succeeded")
    return new_count


def evaluate_alerts(config: FeedConfig) -> list[dict]:
    """Evaluate alert rules against recent items. Returns list of fired alerts."""
    con = get_db()
    fired = []

    for alert in config.alerts:
        if not alert.enabled:
            continue

        # Look up the real alert ID from the database
        workspace_id = alert.workspace_id
        alert_scope = alert.scope
        alert_row = con.execute(
            """SELECT id FROM alerts WHERE name = ? AND scope = ? AND workspace_id = ?""",
            (alert.name, alert_scope, workspace_id),
        ).fetchone()
        if alert_row:
            alert_id = alert_row[0]
        else:
            inserted = con.execute(
                """INSERT INTO alerts (name, match_config, action, deliver_to,
                   hook_command, cooldown_minutes, enabled, scope, workspace_id)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id""",
                (
                    alert.name,
                    json.dumps({
                        "keywords": alert.keywords,
                        "tags": alert.tags,
                        "sources": alert.sources,
                    }),
                    alert.action,
                    alert.deliver_to,
                    alert.hook_command,
                    alert.cooldown_minutes,
                    alert.enabled,
                    alert_scope,
                    workspace_id,
                ),
            ).fetchone()
            alert_id = inserted[0]

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
              AND f.removed_at IS NULL
              AND i.security_status = 'accepted'
              AND (f.scope = 'global' OR f.workspace_id = ?)
            ORDER BY i.ingested_at DESC
            LIMIT 10
        """

        try:
            rows = con.execute(query, [*params, workspace_id]).fetchall()
        except Exception as e:
            logger.error("Alert query error for %s: %s", alert.name, e)
            continue

        for row in rows:
            item_id = row[0]
            already_seen = con.execute(
                "SELECT 1 FROM alert_log WHERE alert_id = ? AND item_id = ? AND success = true LIMIT 1",
                (alert_id, item_id),
            ).fetchone()
            if already_seen:
                continue
            if not check_alert_cooldown_by_name(alert):
                continue

            match_score = _compute_match_score(alert, row)
            match_reasons = _compute_match_reasons(alert, row)

            log_alert(
                alert_id=alert_id, item_id=item_id, match_score=match_score,
                match_reasons=match_reasons, action=alert.action, success=False,
                detail="matched; awaiting host delivery",
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


def fire_alerts(fired_alerts: list[dict]) -> list[dict]:
    """Return delivery intents for the host-owned delivery runtime."""
    return [
        alert for alert in fired_alerts
        if alert["action"] in ("deliver", "both") and alert["deliver_to"]
    ]


def _run_ingestion(workspace: str | None = None, source_name: str | None = None) -> dict:
    """Main ingestion entry point. Returns summary stats."""
    if workspace:
        os.environ["VAK_FEED_WORKSPACE"] = workspace
    config = init_feed_system(workspace)
    sync_alerts(config)
    run_id = begin_ingestion_run(source_name or "")

    start_time = time.time()
    total_new = 0
    sources_ingested = 0
    errors = 0

    for source in config.sources:
        if source_name and source.name != source_name:
            continue
        if not source_name and not source_is_due(
            source.id, source.scope, source.workspace_id
        ):
            logger.info("Skipping source not due: %s", source.name)
            continue

        try:
            mark_source_checked(source)
            count = ingest_source(source)
            sources_ingested += 1
            if count < 0:
                errors += 1
            else:
                total_new += count
        except Exception as e:
            logger.error("Error ingesting %s: %s", source.name, e)
            errors += 1

    fired = []
    try:
        fired = evaluate_alerts(config)
        delivery_intents = fire_alerts(fired)
        unsupported = [alert for alert in fired if alert not in delivery_intents]
        if unsupported:
            errors += len(unsupported)
            for alert in unsupported:
                logger.error(
                    "Alert '%s' matched but has no host delivery target; action=%s",
                    alert.get("alert_name", ""), alert.get("action", ""),
                )
    except Exception as e:
        logger.error("Error evaluating alerts: %s", e)
        errors += 1

    prune_seen(config.dedup_window_days)

    elapsed = time.time() - start_time
    finish_ingestion_run(
        run_id,
        "failed" if errors else "succeeded",
        sources_seen=sources_ingested,
        sources_succeeded=sources_ingested - errors,
        items_added=total_new,
        error=f"{errors} source or alert operation(s) failed" if errors else "",
    )
    summary = {
        "sources_ingested": sources_ingested,
        "new_items": total_new,
        "alerts_fired": len(fired),
        "errors": errors,
        "delivery_intents": delivery_intents,
        "elapsed_seconds": round(elapsed, 2),
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }
    logger.info("Ingestion complete: %s", json.dumps(summary))
    return summary


@contextlib.contextmanager
def _ingestion_lease(workspace: str | None):
    """Serialize ingestion per workspace across scheduler and CLI processes."""
    root = Path(workspace) if workspace else Path.cwd()
    lock_path = root / ".vak" / "feeds.ingest.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+") as lock:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError("another ingestion run is active") from error
        try:
            yield
        finally:
            fcntl.flock(lock.fileno(), fcntl.LOCK_UN)


def run_ingestion(workspace: str | None = None, source_name: str | None = None) -> dict:
    """Run one serialized ingestion and return an operational receipt."""
    try:
        with _ingestion_lease(workspace):
            return _run_ingestion(workspace, source_name)
    except RuntimeError as error:
        logger.info("Ingestion skipped: %s", error)
        return {
            "sources_ingested": 0, "new_items": 0, "alerts_fired": 0,
            "errors": 0, "delivery_intents": [], "skipped": True,
            "reason": str(error), "timestamp": datetime.now(timezone.utc).isoformat(),
        }


def main():
    parser = argparse.ArgumentParser(description="Feed ingestion pipeline")
    parser.add_argument("--workspace", "-w", help="Workspace path")
    parser.add_argument("--source", "-s", help="Ingest a specific source by name")
    parser.add_argument("--init", action="store_true", help="Initialize DB only")
    parser.add_argument("--stats", action="store_true", help="Print stats only")
    parser.add_argument("--sync-alerts", action="store_true", help="Reconcile configured alerts into storage")
    # Admin-console CRUD against the live `feeds` table -- see
    # feed_utils.update_feed_row/remove_feed_row for why this operates on
    # the DB directly instead of feeds.toml (which is what the UI
    # actually displays and what update/remove in
    # crates/vak-server/src/feeds.rs call through to). Not exposed via
    # feed_mcp.py's tool list: that surface is reachable by LLM agents,
    # and source removal shouldn't be an agent-callable tool.
    parser.add_argument("--remove-source", metavar="SOURCE_ID", help="Soft-delete by stable source id")
    parser.add_argument("--update-source", metavar="SOURCE_ID", help="Update by stable source id")
    parser.add_argument("--scope", choices=["global", "workspace"], help="Source scope for admin operations")
    parser.add_argument("--release-item", type=int, help="Release a quarantined item")
    parser.add_argument("--mark-alert-delivered", nargs=2, metavar=("ALERT_ID", "ITEM_ID"))
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

    if args.sync_alerts:
        init_feed_system(args.workspace)
        config = load_feed_config(args.workspace)
        print(json.dumps({"status": "ok", "alerts": sync_alerts(config)}))
        return

    if args.remove_source:
        init_feed_system(args.workspace)
        ok = remove_feed_row(args.remove_source, args.scope)
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
            scope=args.scope,
        )
        print(json.dumps({"status": "ok" if ok else "not_found", "name": args.update_source}))
        sys.exit(0 if ok else 1)

    if args.release_item:
        init_feed_system(args.workspace)
        ok = set_item_security_status(args.release_item, "accepted", "released by operator")
        print(json.dumps({"status": "ok" if ok else "not_found", "id": args.release_item}))
        sys.exit(0 if ok else 1)

    if args.mark_alert_delivered:
        init_feed_system(args.workspace)
        alert_id, item_id = (int(value) for value in args.mark_alert_delivered)
        ok = mark_alert_delivered(alert_id, item_id)
        print(json.dumps({"status": "ok" if ok else "not_found", "alert_id": alert_id, "item_id": item_id}))
        sys.exit(0 if ok else 1)

    summary = run_ingestion(args.workspace, args.source)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
