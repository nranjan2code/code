#!/bin/sh
# Two machines: "desk" and "away", each with its own data home, sharing /remote.
set -e
docker rm -f lab-desk lab-away >/dev/null 2>&1 || true
docker volume rm -f lab-desk-data lab-away-data lab-remote >/dev/null 2>&1 || true
for m in desk away; do
  docker run -d --name lab-$m --hostname $m \
    -v vk-target:/target:ro -v lab-$m-data:/data -v lab-remote:/remote \
    -e VAK_HOME=/data/vak -e HOME=/data/home -e VAK_GATEWAY_TOKEN=lab-token-$m \
    -e VAK_OLLAMA_BASE_URL=http://host.docker.internal:11434 \
    -e VAK_KEY_PASSPHRASE="four quiet lanterns" \
    rust:1-bookworm sleep infinity >/dev/null
  docker exec lab-$m sh -c 'mkdir -p /data/vak /data/home /work/.vak /data/vak/vak-home/.vak && ln -sf /target/debug/vak /usr/local/bin/vak && printf "provider = \"ollama\"\nmodel = \"gemma4:e2b-mlx\"\n\n[lifecycle]\nmode = \"commit\"\n" > /work/.vak/config.toml && cp /work/.vak/config.toml /data/vak/vak-home/.vak/config.toml'
done
docker exec lab-desk sh -c 'curl -s -m 5 http://host.docker.internal:11434/api/tags | head -c 120; echo; vak --version'
