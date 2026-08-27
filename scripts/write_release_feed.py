#!/usr/bin/env python3
"""Merge this host's artifact into the update feed for one release version.

`vakcoder self update <url>` fetches this JSON, picks the entry matching
`<os>/<arch>`, downloads the bare binary, and refuses bytes whose SHA-256 does
not match. Release builds happen one platform per host, so each run merges its
own key instead of replacing the file.
"""

import json
import os
from pathlib import Path

feed = Path(os.environ["VAKCODER_FEED"])
version = os.environ["VAKCODER_VERSION"]

manifest = {"version": version, "artifacts": {}}
if feed.exists():
    existing = json.loads(feed.read_text())
    # A feed left over from a different version must not keep other
    # platforms' stale artifacts alive under the new version number.
    if existing.get("version") == version:
        manifest = existing
        manifest.setdefault("artifacts", {})

manifest["version"] = version
manifest["artifacts"][os.environ["VAKCODER_KEY"]] = {
    "url": os.environ["VAKCODER_URL"],
    "sha256": os.environ["VAKCODER_SHA256"],
}

feed.parent.mkdir(parents=True, exist_ok=True)
feed.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
print(f"update feed: {feed} [{os.environ['VAKCODER_KEY']}]")
