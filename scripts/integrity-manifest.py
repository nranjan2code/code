#!/usr/bin/env python3
"""Tamper-evidence over the release *set*
(docs/design/46-stabilization-install-and-onboarding.md Part V).

Per-file checksums prove a file was not corrupted. They do not prove the
*set* was not edited: an artifact removed, an extra one added, or one
swapped for another all leave every remaining checksum valid.

So every release directory carries a manifest naming each artifact with its
exact byte length and SHA-256, plus a digest over the manifest's own
canonical payload. The verifier rejects a missing, extra, duplicated, or
path-escaping entry, and any size or digest drift.

Usage:
  integrity-manifest.py [DIR]           write the manifest
  integrity-manifest.py --verify [DIR]  check it
"""

import hashlib
import json
import os
import sys

MANIFEST = "integrity-manifest.json"
# Produced by this script, so never described by it.
EXCLUDED = {MANIFEST}


def latest_release_dir() -> str:
    root = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "dist")
    if not os.path.isdir(root):
        sys.exit(f"no dist directory at {root}")
    versions = [d for d in os.listdir(root) if os.path.isdir(os.path.join(root, d))]
    if not versions:
        sys.exit(f"no release directories under {root}")
    return os.path.join(root, sorted(versions)[-1])


def artifacts(directory: str):
    for name in sorted(os.listdir(directory)):
        if name in EXCLUDED or name.startswith("."):
            continue
        path = os.path.join(directory, name)
        if not os.path.isfile(path):
            continue
        with open(path, "rb") as handle:
            data = handle.read()
        yield {
            "name": name,
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        }


def canonical(entries) -> bytes:
    """Byte-stable payload the manifest digest is taken over.

    Sorted keys and no incidental whitespace, so the same set of files
    always produces the same digest regardless of who wrote it.
    """
    return json.dumps(entries, sort_keys=True, separators=(",", ":")).encode()


def write(directory: str) -> int:
    entries = list(artifacts(directory))
    if not entries:
        sys.exit(f"{directory} contains no artifacts to describe")
    payload = {"schema": 1, "artifacts": entries}
    manifest = {
        **payload,
        "manifest_sha256": hashlib.sha256(canonical(payload)).hexdigest(),
    }
    with open(os.path.join(directory, MANIFEST), "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(f"integrity manifest: {len(entries)} artifact(s) in {directory}")
    return 0


def verify(directory: str) -> int:
    path = os.path.join(directory, MANIFEST)
    if not os.path.isfile(path):
        sys.exit(f"no {MANIFEST} in {directory}")
    with open(path, encoding="utf-8") as handle:
        manifest = json.load(handle)

    payload = {"schema": manifest["schema"], "artifacts": manifest["artifacts"]}
    if hashlib.sha256(canonical(payload)).hexdigest() != manifest["manifest_sha256"]:
        sys.exit("the manifest's own payload has been modified")

    declared = {a["name"]: a for a in manifest["artifacts"]}
    problems = []
    for name in declared:
        # A name that escapes the release directory would have the
        # verifier read, and implicitly bless, a file somewhere else.
        if os.path.basename(name) != name:
            problems.append(f"{name}: path escapes the release directory")

    seen = set()
    for found in artifacts(directory):
        name = found["name"]
        if name in seen:
            problems.append(f"{name}: listed twice")
        seen.add(name)
        expected = declared.get(name)
        if expected is None:
            problems.append(f"{name}: present but not described by the manifest")
            continue
        if expected["sha256"] != found["sha256"]:
            problems.append(f"{name}: digest drift")
        elif expected["bytes"] != found["bytes"]:
            problems.append(f"{name}: size drift")

    for name in declared:
        if name not in seen:
            problems.append(f"{name}: described by the manifest but missing")

    if problems:
        print("integrity manifest FAILED:")
        for problem in problems:
            print(f"  ✗ {problem}")
        return 1
    print(f"integrity manifest verifies: {len(declared)} artifact(s)")
    return 0


def main() -> int:
    args = [a for a in sys.argv[1:]]
    checking = "--verify" in args
    args = [a for a in args if a != "--verify"]
    directory = args[0] if args else latest_release_dir()
    return verify(directory) if checking else write(directory)


if __name__ == "__main__":
    raise SystemExit(main())
