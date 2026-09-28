#!/usr/bin/env bash
# Build standalone arm64 Linux binaries locally with Docker BuildKit.
# The source context contains tracked build inputs only; local configuration,
# credentials, .vak state, Git metadata, and unrelated files are excluded.
#
# Usage:
#   scripts/build-amazonlinux-arm64.sh [output-directory]

set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

command -v docker >/dev/null || { printf 'Docker is required.\n' >&2; exit 1; }
docker buildx version >/dev/null 2>&1 || {
    printf 'Docker Buildx is required. Start Docker Desktop and try again.\n' >&2
    exit 1
}

if [[ -n "$(git status --porcelain)" ]]; then
    printf 'Commit or stash changes before building so the build stamp names its source.\n' >&2
    exit 1
fi

GIT_SHA="$(git rev-parse --short HEAD)"
OUTPUT_DIR="${1:-$ROOT_DIR/target/amazonlinux-arm64}"
mkdir -p "$OUTPUT_DIR"
BUILD_CONTEXT="$(mktemp -d "${TMPDIR:-/tmp}/vak-arm64-build.XXXXXX")"
trap 'rm -rf "$BUILD_CONTEXT"' EXIT

# Stage the minimal source inputs from Git. In particular, never send the
# working directory, .env files, .vak configuration, SSH keys, or AWS profile
# files to Docker, even if one happens to sit beside the repository.
python3 - "$ROOT_DIR" "$BUILD_CONTEXT" <<'PY'
import pathlib
import shutil
import subprocess
import sys

root = pathlib.Path(sys.argv[1])
context = pathlib.Path(sys.argv[2])
tracked = subprocess.run(
    ["git", "ls-files", "-z"], cwd=root, check=True, stdout=subprocess.PIPE
).stdout.split(b"\0")
required_root = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "scripts/ui_bundle_check.rs",
}
paths = [
    item.decode()
    for item in tracked
    if item
    and (item.startswith(b"crates/") or item.decode() in required_root)
]
missing = [name for name in paths if not (root / name).exists()]
if missing:
    raise SystemExit("tracked build input is missing: " + ", ".join(missing[:5]))

for name in paths:
    source = root / name
    target = context / name
    target.parent.mkdir(parents=True, exist_ok=True)
    if source.is_symlink():
        target.symlink_to(source.readlink())
    elif source.is_file():
        shutil.copy2(source, target)
PY

cp "$ROOT_DIR/docker/Dockerfile.amazonlinux-build" "$BUILD_CONTEXT/Dockerfile"

printf 'Building vak %s for Linux arm64 (Amazon Linux 2023)...\n' "$GIT_SHA"
docker buildx build \
    --platform linux/arm64 \
    --file "$BUILD_CONTEXT/Dockerfile" \
    --build-arg "VAK_GIT_SHA=$GIT_SHA" \
    --target export \
    --output "type=local,dest=$OUTPUT_DIR" \
    "$BUILD_CONTEXT"

for binary in vak vak-delivery-worker; do
    if [[ ! -x "$OUTPUT_DIR/$binary" ]]; then
        printf 'Build output is missing executable %s.\n' "$binary" >&2
        exit 1
    fi
done

# Keep the provenance and digests beside the ignored artifacts. Deployers
# refuse a mismatched checkout or modified binary instead of shipping an
# arbitrary old target/ file under the current source revision.
python3 - "$OUTPUT_DIR" "$(git rev-parse HEAD)" <<'PY'
import hashlib
import json
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
manifest = {
    "source_commit": sys.argv[2],
    "sha256": {
        name: hashlib.sha256((out / name).read_bytes()).hexdigest()
        for name in ("vak", "vak-delivery-worker")
    },
}
(out / "build-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
PY

file "$OUTPUT_DIR/vak" "$OUTPUT_DIR/vak-delivery-worker"
printf 'Binaries written to %s\n' "$OUTPUT_DIR"
