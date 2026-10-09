#!/bin/sh
# One machine is this Mac, the other a Linux container; the remote is a
# folder on the Mac that the container mounts (data-architecture plan M9).
# Run from the repository root after `cargo build -p vak -p vak-server
# --bins` and `sh scripts/sync-lab/build.sh`. Needs Docker and Ollama with
# gemma4:e2b-mlx. Everything lives under /tmp/vak-mac-lab; the operator's
# own data home is never touched. The passphrase is a throwaway test value.
set -u
ROOT=/tmp/vak-mac-lab
MAC="$PWD/target/debug/vak"
PASS=0
FAIL=0
check() {
  if [ "$2" = 0 ]; then PASS=$((PASS + 1)); echo "PASS  $1  |  $3"; else FAIL=$((FAIL + 1)); echo "FAIL  $1  |  $3"; fi
}
docker rm -f lab-linux >/dev/null 2>&1
rm -rf "$ROOT"
mkdir -p "$ROOT/home/fake" "$ROOT/work/.vak" "$ROOT/remote" "$ROOT/home/vak-home/.vak"
printf 'provider = "ollama"\nmodel = "gemma4:e2b-mlx"\n' > "$ROOT/work/.vak/config.toml"
cp "$ROOT/work/.vak/config.toml" "$ROOT/home/vak-home/.vak/config.toml"
export VAK_KEY_PASSPHRASE="four quiet lanterns"
mac() { (cd "$ROOT/work" && VAK_HOME="$ROOT/home" HOME="$ROOT/home/fake" "$MAC" "$@"); }

docker run -d --name lab-linux --hostname linux \
  -v vk-target:/target:ro -v "$ROOT/remote":/remote \
  -e VAK_HOME=/data/vak -e HOME=/data/home \
  -e VAK_OLLAMA_BASE_URL=http://host.docker.internal:11434 \
  -e VAK_KEY_PASSPHRASE="$VAK_KEY_PASSPHRASE" \
  rust:1-bookworm sleep infinity >/dev/null
docker exec lab-linux sh -c 'mkdir -p /data/vak/vak-home/.vak /data/home /work/.vak && ln -sf /target/debug/vak /usr/local/bin/vak && printf "provider = \"ollama\"\nmodel = \"gemma4:e2b-mlx\"\n" > /work/.vak/config.toml && cp /work/.vak/config.toml /data/vak/vak-home/.vak/config.toml'
linux() { docker exec -i -w /work lab-linux vak "$@"; }

# 1. A real turn on the Mac, pushed to the folder.
mac exec --trust "Reply in one short sentence: the marigold budget is due Monday." > "$ROOT/turn1.txt" 2>&1
check "1a a real turn on the Mac" $? "$(head -c 80 "$ROOT/turn1.txt" | tr '\n' ' ')"
SID=$(mac sessions 2>/dev/null | awk '/^[0-9a-f]{8}-/ {print $1; exit}')
mac sync setup "$ROOT/remote" >/dev/null 2>&1
OUT=$(mac sync now 2>&1); check "1b the Mac pushes to a folder" $? "$(echo "$OUT" | head -1)"

# 2. The key file carried by hand, then Linux reads the same conversation.
mac sync key export "$ROOT/key.txt" >/dev/null 2>&1
docker cp -q "$ROOT/key.txt" lab-linux:/data/key.txt
linux sync key import /data/key.txt >/dev/null 2>&1
linux sync setup /remote >/dev/null 2>&1
OUT=$(linux sync pull 2>&1); check "2a Linux pulls what the Mac pushed" $? "$(echo "$OUT" | head -1)"
linux data cat "$SID" 2>/dev/null | grep -qi marigold
check "2b Linux reads the Mac's conversation" $? "session ${SID%%-*}"
OUT=$(linux data verify 2>&1); check "2c the Linux copy verifies" $? "$(echo "$OUT" | head -1)"

# 3. The Mac hands over; Linux takes over, works, and pushes.
OUT=$(mac sync handover 2>&1); check "3a the Mac hands over" $? "$(echo "$OUT" | head -1)"
OUT=$(linux sync takeover 2>&1); check "3b Linux takes over" $? "$(echo "$OUT" | head -1)"
docker exec -i -w /work lab-linux vak exec --trust "Reply in one short sentence: the juniper order ships Tuesday." > "$ROOT/turn2.txt" 2>&1
check "3c a real turn on Linux" $? "$(head -c 80 "$ROOT/turn2.txt" | tr '\n' ' ')"
SID2=$(linux sessions 2>/dev/null | awk '/^[0-9a-f]{8}-/ {print $1}' | grep -v "$SID" | head -1)
OUT=$(linux sync now 2>&1); check "3d Linux pushes" $? "$(echo "$OUT" | head -1)"

# 4. The Mac takes the work back and reads what Linux did.
OUT=$(linux sync handover 2>&1); check "4a Linux hands back" $? "$(echo "$OUT" | head -1)"
OUT=$(mac sync takeover 2>&1); check "4b the Mac takes the work back" $? "$(echo "$OUT" | head -1)"
mac data cat "$SID2" 2>/dev/null | grep -qi juniper
check "4c the Mac reads Linux's conversation" $? "session ${SID2%%-*}"
mac data cat "$SID" 2>/dev/null | grep -qi marigold
check "4d and still its own" $? "session ${SID%%-*}"
OUT=$(mac data verify 2>&1); check "4e the Mac copy verifies" $? "$(echo "$OUT" | head -1)"
N=$(find "$ROOT/remote" -type f | wc -l | tr -d ' ')
U=$(find "$ROOT/remote" -type f | tr 'A-Z' 'a-z' | sort | uniq -d | wc -l | tr -d ' ')
[ "$U" = 0 ]; check "4f no two remote files differ only by case" $? "$N files in the remote"

docker rm -f lab-linux >/dev/null 2>&1
echo
echo "$PASS of $((PASS + FAIL)) checks passed"
[ "$FAIL" = 0 ]
