#!/usr/bin/env python3
"""Merge every matrix leg's release.json into one feed
(docs/design/46-stabilization-install-and-onboarding.md S8).

Each leg builds for one platform and writes a feed naming only its own
key. An install on any other platform reading that feed sees a release
that offers it nothing — which is the state a laptop-run release left
every non-laptop platform in.

Usage: merge-feeds.py LEGS_DIR OUT_DIR
"""

import json
import os
import shutil
import sys


def main() -> int:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    legs_dir, out_dir = sys.argv[1], sys.argv[2]

    feeds = []
    for root, _dirs, files in os.walk(legs_dir):
        if "release.json" in files:
            with open(os.path.join(root, "release.json"), encoding="utf-8") as handle:
                feeds.append((root, json.load(handle)))
    if not feeds:
        sys.exit(f"no release.json found under {legs_dir}")

    versions = {feed["version"] for _, feed in feeds}
    if len(versions) != 1:
        # Legs that disagree about the version would produce a feed where
        # a platform's artifacts belong to a different release than the
        # one the feed names.
        sys.exit(f"legs built different versions: {sorted(versions)}")
    version = versions.pop()

    platforms = {}
    for source, feed in feeds:
        for key, value in feed["platforms"].items():
            if key in platforms:
                sys.exit(f"two legs both claim platform {key}")
            platforms[key] = value
        # The artifacts travel with the feed that describes them.
        target = os.path.join(out_dir, version)
        os.makedirs(target, exist_ok=True)
        for name in os.listdir(source):
            if name == "release.json":
                continue
            src = os.path.join(source, name)
            if os.path.isfile(src):
                shutil.copy2(src, os.path.join(target, name))

    merged = {
        "schema": 2,
        "version": version,
        "released_at": max(feed["released_at"] for _, feed in feeds),
        "platforms": platforms,
    }
    target = os.path.join(out_dir, version)
    os.makedirs(target, exist_ok=True)
    with open(os.path.join(target, "release.json"), "w", encoding="utf-8") as handle:
        json.dump(merged, handle, indent=2, sort_keys=True)
        handle.write("\n")

    print(f"merged {len(feeds)} leg(s) into one feed for {version}")
    for key in sorted(platforms):
        print(f"  {key}: {len(platforms[key]['components'])} component(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
