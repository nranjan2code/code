#!/bin/sh
# One machine with a fresh data home, the fake provider on its loopback, and
# the soak's server. The client ids and token are throwaway test values.
set -e
docker rm -f mail-soak >/dev/null 2>&1 || true
docker volume rm -f mail-soak-data >/dev/null 2>&1 || true
docker run -d --name mail-soak --hostname soak \
  -v vk-soak-target:/target:ro -v mail-soak-data:/data -v "$(cd "$(dirname "$0")" && pwd)":/soak:ro \
  -e VAK_HOME=/data/vak -e HOME=/data/home -e VAK_GATEWAY_TOKEN=soak-token \
  -e VAK_OLLAMA_BASE_URL=http://host.docker.internal:11434 \
  -e VAK_TEST_PROVIDER_BASE=http://127.0.0.1:9100 \
  -e VAK_GOOGLE_OAUTH_CLIENT_ID=soak-google-client \
  -e FAKE_HOME=/data/fake \
  rust:1-bookworm sleep infinity >/dev/null
docker exec mail-soak sh -c 'mkdir -p /data/vak/vak-home/.vak /data/home /work/.vak && ln -sf /target/debug/vak /usr/local/bin/vak && printf "provider = \"ollama\"\nmodel = \"gemma4:e2b-mlx\"\n\n[lifecycle]\nmode = \"commit\"\n" > /work/.vak/config.toml && cp /work/.vak/config.toml /data/vak/vak-home/.vak/config.toml'
docker exec -d mail-soak sh -c 'python3 /soak/fake.py >> /data/fake.log 2>&1'
echo up
