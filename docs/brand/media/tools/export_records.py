# Rebuilds elevenlabs/generations.json from saved results of the connector's
# creative_get_flow_run_status, one record per generation:
#
#   python3 -I export_records.py <saved result>...
#
# Signed download links and the workspace code are dropped; the run fails if
# any survive, so the record never grants access to the account.
import json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
MEDIA = os.path.join(HERE, "..")
SECRET = re.compile(r"X-Goog|oobCode|[0-9a-f]{32}")


def session_map():
    source = open(os.path.join(HERE, "fetch_flow_results.py")).read()
    namespace = {}
    exec(source.split('if sys.argv[1]=="ids":')[0], namespace)
    return namespace["MAP"]


def main(paths):
    mapping = session_map()
    out_path = os.path.join(MEDIA, "elevenlabs", "generations.json")
    records = {r["generation_id"]: r for r in json.load(open(out_path))["generations"]} if os.path.exists(out_path) else {}
    for path in paths:
        data = json.load(open(path))
        settings = {m["generation_id"]: m.get("settings") for m in data.get("media", [])}
        prompts = {m["generation_id"]: m.get("prompt") for m in data.get("media", [])}
        for g in data.get("generations", []):
            found = re.search(r"content_generation/([^/]+)/", g.get("content_url", ""))
            session = found.group(1) if found else None
            base, take, takes = mapping.get(session, ("unmapped", 1, 1))
            records[g["id"]] = {
                "generation_id": g["id"], "session_id": session,
                "file": base + (f"-{take}" if takes > 1 else ""),
                "model_id": g.get("model_id"), "modality": g.get("modality"),
                "prompt": prompts.get(g["id"]) or g.get("prompt"), "settings": settings.get(g["id"]),
                "seconds": g.get("duration_secs"), "created_at_unix_ms": g.get("created_at_unix_ms"),
                "credits": (g.get("price") or {}).get("credits"),
            }
    ordered = sorted(records.values(), key=lambda r: r["created_at_unix_ms"] or 0)
    text = json.dumps({"flow": {"name": "Vakyartha brand assets 2026-10", "id": "uutJRnMAgX5LIOXroskb"},
                       "note": "Exported from the ElevenLabs connector's run status. Signed download links "
                               "and workspace codes are removed on purpose.",
                       "generations": ordered}, indent=2, ensure_ascii=False)
    if SECRET.search(text):
        sys.exit("refusing to write: a link or workspace code survived")
    open(out_path, "w").write(text + "\n")
    print(len(ordered), "generations,", round(sum(r["credits"] or 0 for r in ordered)), "credits")


if __name__ == "__main__":
    main(sys.argv[1:])
