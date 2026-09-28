#!/usr/bin/env bash
# Build standalone arm64 Linux binaries locally with Docker BuildKit.
# The source context contains only regular tracked blobs from the selected
# commit; credentials, Git metadata and unrelated files never enter Docker.
# Usage: scripts/build-amazonlinux-arm64.sh [full-source-commit]
# Artifacts are retained at target/amazonlinux-arm64/<commit>/<builder-sha>/.

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

GIT_COMMIT="${1:-$(git rev-parse HEAD)}"
if [[ ! "$GIT_COMMIT" =~ ^[0-9a-f]{40}$ ]] || ! git cat-file -e "$GIT_COMMIT^{commit}"; then
    printf 'Build source must be a full 40-character Git commit present in this checkout.\n' >&2
    exit 1
fi
GIT_SHA="$(git rev-parse --short "$GIT_COMMIT")"
BUILDER_DOCKERFILE_SHA="$(git show HEAD:docker/Dockerfile.amazonlinux-build | python3 -c 'import hashlib,sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())')"
BUILDX_VERSION="$(docker buildx version)"
ARTIFACT_ROOT="$ROOT_DIR/target/amazonlinux-arm64"
OUTPUT_DIR="$ARTIFACT_ROOT/$GIT_COMMIT/$BUILDER_DOCKERFILE_SHA"
if [[ -e "$OUTPUT_DIR" ]]; then
    printf 'Artifacts already exist for %s; deploy or inspect the retained artifact instead of overwriting it.\n' "$GIT_COMMIT" >&2
    exit 1
fi
mkdir -p "$ARTIFACT_ROOT/$GIT_COMMIT"
STAGING_DIR="$(mktemp -d "$ARTIFACT_ROOT/.build-$GIT_COMMIT.XXXXXXXX")"
OUTPUT_STAGE="$STAGING_DIR/output"
mkdir -p "$OUTPUT_STAGE"
BUILD_CONTEXT="$(mktemp -d "${TMPDIR:-/tmp}/vak-arm64-build.XXXXXX")"
cleanup() { rm -rf "$BUILD_CONTEXT" "$STAGING_DIR"; }
trap cleanup EXIT

# Materialize immutable Git blobs instead of reading the working tree. This
# makes an explicit rollback build use precisely that commit's source while
# still using the current committed builder definition.
python3 - "$ROOT_DIR" "$BUILD_CONTEXT" "$GIT_COMMIT" <<'PY'
import pathlib
import subprocess
import sys

root = pathlib.Path(sys.argv[1])
context = pathlib.Path(sys.argv[2])
commit = sys.argv[3]
required_root = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "scripts/ui_bundle_check.rs",
}
entries = subprocess.run(
    ["git", "ls-tree", "-rz", "-r", commit],
    cwd=root,
    check=True,
    stdout=subprocess.PIPE,
).stdout.split(b"\0")
seen = set()
for entry in entries:
    if not entry:
        continue
    metadata, raw_name = entry.split(b"\t", 1)
    mode, object_type, object_id = metadata.split(b" ", 2)
    name = raw_name.decode("utf-8")
    if not (name.startswith("crates/") or name in required_root):
        continue
    if object_type != b"blob" or mode == b"120000":
        raise SystemExit(f"Build input must be a regular tracked file: {name}")
    data = subprocess.run(
        ["git", "cat-file", "blob", object_id.decode("ascii")],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    target = context / name
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    seen.add(name)
missing = sorted(required_root - seen)
if missing:
    raise SystemExit("required build input is missing from source commit: " + ", ".join(missing))
PY

# Use the current committed builder definition for current and rollback builds.
git show HEAD:docker/Dockerfile.amazonlinux-build > "$BUILD_CONTEXT/Dockerfile"

printf 'Building vak %s for Linux arm64 (Amazon Linux 2023)...\n' "$GIT_SHA"
docker buildx build \
    --progress=plain \
    --platform linux/arm64 \
    --file "$BUILD_CONTEXT/Dockerfile" \
    --build-arg "VAK_GIT_SHA=$GIT_SHA" \
    --target export \
    --output "type=local,dest=$OUTPUT_STAGE" \
    "$BUILD_CONTEXT"

for binary in vak vak-delivery-worker; do
    if [[ ! -x "$OUTPUT_STAGE/$binary" ]]; then
        printf 'Build output is missing executable %s.\n' "$binary" >&2
        exit 1
    fi
done
if [[ ! -s "$OUTPUT_STAGE/build-environment.txt" ]]; then
    printf 'Build output is missing builder environment evidence.\n' >&2
    exit 1
fi
file "$OUTPUT_STAGE/vak" "$OUTPUT_STAGE/vak-delivery-worker"
python3 - "$OUTPUT_STAGE/vak" "$OUTPUT_STAGE/vak-delivery-worker" <<'PY'
import pathlib
import struct
import sys

for name in sys.argv[1:]:
    header = pathlib.Path(name).read_bytes()[:20]
    if len(header) < 20 or header[:4] != b"\x7fELF" or header[4] != 2 or header[5] != 1:
        raise SystemExit(f"Build output is not a 64-bit little-endian ELF file: {pathlib.Path(name).name}")
    machine = struct.unpack_from("<H", header, 18)[0]
    if machine != 183:
        raise SystemExit(f"Build output is not an AArch64 ELF executable: {pathlib.Path(name).name}")
PY

# Provenance binds source, lockfile, builder definition, runtime environment,
# and both deployable binary digests.
python3 - "$OUTPUT_STAGE" "$GIT_COMMIT" "$BUILD_CONTEXT" "$BUILDX_VERSION" <<'PY'
import hashlib
import json
import pathlib
import re
import sys

out = pathlib.Path(sys.argv[1])
commit = sys.argv[2]
context = pathlib.Path(sys.argv[3])
lock = (context / "Cargo.lock").read_bytes()
dockerfile = (context / "Dockerfile").read_bytes()
text = dockerfile.decode("utf-8")
image = re.search(r"^FROM (\S+) AS builder$", text, re.MULTILINE)
release = re.search(r"^ARG AMAZON_LINUX_RELEASE=(\S+)$", text, re.MULTILINE)
toolchain = re.search(r"^ENV RUSTUP_TOOLCHAIN=(\S+)$", text, re.MULTILINE)
if not image or not release or not toolchain:
    raise SystemExit("builder Dockerfile is missing pinned image, repository release, or Rust toolchain")
manifest = {
    "source_commit": commit,
    "target": "aarch64-unknown-linux-gnu",
    "amazon_linux_release": release.group(1),
    "builder_image": image.group(1),
    "rust_toolchain": toolchain.group(1),
    "cargo_lock_sha256": hashlib.sha256(lock).hexdigest(),
    "builder_dockerfile_sha256": hashlib.sha256(dockerfile).hexdigest(),
    "docker_buildx_version": sys.argv[4],
    "builder_environment_sha256": hashlib.sha256((out / "build-environment.txt").read_bytes()).hexdigest(),
    "sha256": {
        name: hashlib.sha256((out / name).read_bytes()).hexdigest()
        for name in ("vak", "vak-delivery-worker")
    },
}
(out / "build-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
PY

mv "$OUTPUT_STAGE" "$OUTPUT_DIR"
printf 'Build manifest:\n'
cat "$OUTPUT_DIR/build-manifest.json"
printf 'Binaries retained at %s\n' "$OUTPUT_DIR"
