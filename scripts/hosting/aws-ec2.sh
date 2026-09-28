#!/usr/bin/env bash
# Operate one private-inventory AWS EC2 deployment of the headless Vakyartha
# release. No account, host, domain, token, or key is embedded in this file.
# Usage: scripts/hosting/aws-ec2.sh /outside/repo/aws.env <command>
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
CONFIG="${1:-}"
ACTION="${2:-}"
usage() {
    printf 'Usage: %s /outside/repo/aws.env {build|deploy|setup|status|logs|tunnel|copy-token|proxy|web-check}\n' "$0" >&2
    exit 2
}
[[ -n "$CONFIG" && -n "$ACTION" && $# -eq 2 ]] || usage
[[ -f "$CONFIG" ]] || { printf 'Private inventory file is missing.\n' >&2; exit 1; }

# The inventory is deliberately a local, trusted shell file. Never source one
# from the checkout, since the public repository cannot own deployment data.
CONFIG="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve(strict=True))' "$CONFIG")"
case "$CONFIG" in "$ROOT"/*) printf 'Keep the inventory outside the Git checkout.\n' >&2; exit 1 ;; esac
python3 - "$CONFIG" <<'PY'
import os
import stat
import sys

mode = stat.S_IMODE(os.stat(sys.argv[1]).st_mode)
if mode & 0o077:
    raise SystemExit("Private inventory must not be readable by group or others; chmod 600 it")
PY
# shellcheck source=/dev/null
source "$CONFIG"

: "${AWS_PROFILE:?Set AWS_PROFILE in the private inventory}"
: "${AWS_REGION:?Set AWS_REGION in the private inventory}"
: "${INSTANCE_ID:?Set INSTANCE_ID in the private inventory}"
: "${SSH_USER:?Set SSH_USER in the private inventory}"
: "${SSH_KEY:?Set SSH_KEY in the private inventory}"
[[ "$AWS_REGION" =~ ^[a-z0-9-]+$ && "$INSTANCE_ID" =~ ^i-[0-9a-f]+$ && "$SSH_USER" =~ ^[a-z_][a-z0-9_-]*$ ]] || {
    printf 'Inventory has an invalid region, instance ID, or SSH user.\n' >&2; exit 1;
}
[[ -f "$SSH_KEY" ]] || { printf 'SSH private key is missing.\n' >&2; exit 1; }
command -v aws >/dev/null || { printf 'AWS CLI is required.\n' >&2; exit 1; }

aws_ec2() { aws --profile "$AWS_PROFILE" --region "$AWS_REGION" ec2 "$@"; }
host_ip() {
    local ip
    ip="$(aws_ec2 describe-instances --instance-ids "$INSTANCE_ID" \
        --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)"
    [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
        printf 'Instance has no public IPv4 address. Is it running?\n' >&2; exit 1;
    }
    printf '%s' "$ip"
}
remote() {
    local ip="$1"
    shift
    ssh -i "$SSH_KEY" -o BatchMode=yes -o StrictHostKeyChecking=yes \
        "$SSH_USER@$ip" "$@"
}
copy_to() {
    local ip="$1" source="$2" destination="$3"
    scp -q -i "$SSH_KEY" -o BatchMode=yes -o StrictHostKeyChecking=yes \
        "$source" "$SSH_USER@$ip:$destination"
}
require_host() {
    : "${PUBLIC_HOST:?Set PUBLIC_HOST in the private inventory}"
    [[ "$PUBLIC_HOST" =~ ^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$ && "$PUBLIC_HOST" == *.* && "$PUBLIC_HOST" != *..* ]] || {
        printf 'PUBLIC_HOST must be a plain DNS hostname.\n' >&2; exit 1;
    }
}

case "$ACTION" in
    build)
        "$ROOT/scripts/build-amazonlinux-arm64.sh"
        ;;
    deploy)
        artifacts="$ROOT/target/amazonlinux-arm64"
        for name in vak vak-delivery-worker; do
            [[ -x "$artifacts/$name" ]] || {
                printf 'Missing %s; run the build command first.\n' "$name" >&2; exit 1;
            }
        done
        [[ -z "$(git -C "$ROOT" status --porcelain)" ]] || {
            printf 'Commit or stash source changes before deploying.\n' >&2; exit 1;
        }
        python3 - "$artifacts" "$(git -C "$ROOT" rev-parse HEAD)" <<'PY'
import hashlib
import json
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
manifest = json.loads((out / "build-manifest.json").read_text())
if manifest["source_commit"] != sys.argv[2]:
    raise SystemExit("Built artifact is from another commit; rebuild first")
for name in ("vak", "vak-delivery-worker"):
    digest = hashlib.sha256((out / name).read_bytes()).hexdigest()
    if digest != manifest["sha256"].get(name):
        raise SystemExit(f"Built artifact {name} has changed; rebuild first")
PY
        ip="$(host_ip)"
        stage="$(remote "$ip" 'mktemp -d "$HOME/.vak-deploy.XXXXXXXX"')"
        [[ "$stage" =~ ^/[^[:space:]]+\.vak-deploy\.[A-Za-z0-9]+$ ]] || {
            printf 'Remote staging path was unexpected.\n' >&2; exit 1;
        }
        copy_to "$ip" "$artifacts/vak" "$stage/vak"
        copy_to "$ip" "$artifacts/vak-delivery-worker" "$stage/vak-delivery-worker"
        remote "$ip" "bash -s -- $(printf '%q' "$stage")" <<'REMOTE'
set -Eeuo pipefail
stage="$1"
trap 'rm -rf -- "$stage"' EXIT
prefix="$HOME/.local/share/vak/local/release"
if systemctl --user is-enabled vak-gateway.service >/dev/null 2>&1; then
    existing_service=yes
else
    existing_service=no
fi
"$stage/vak" self install --force --prefix "$prefix"
"$prefix/bin/vak" self verify --prefix "$prefix"
if [[ "$existing_service" == yes ]]; then
    "$prefix/bin/vak" self services-sync --prefix "$prefix"
    "$prefix/bin/vak" self status --prefix "$prefix"
else
    printf 'Installed but not activated. Run the setup command next.\n'
fi
REMOTE
        printf 'Managed release installed. Existing service reconciled if one was registered.\n'
        ;;
    setup)
        ip="$(host_ip)"
        ssh -tt -i "$SSH_KEY" -o StrictHostKeyChecking=yes \
            "$SSH_USER@$ip" '"$HOME/.local/share/vak/local/release/bin/vak" setup --terminal -C "$HOME/vak-home"'
        ;;
    status)
        ip="$(host_ip)"
        aws_ec2 describe-instances --instance-ids "$INSTANCE_ID" \
            --query 'Reservations[0].Instances[0].[State.Name,InstanceType,Architecture,PublicIpAddress,SecurityGroups[0].GroupId]' --output text
        remote "$ip" 'bash -s' <<'REMOTE'
set -Eeuo pipefail
prefix="$HOME/.local/share/vak/local/release"
"$prefix/bin/vak" self status --prefix "$prefix"
printf 'Gateway: '
systemctl --user is-active vak-gateway.service
printf 'Caddy: '
systemctl is-active caddy.service 2>/dev/null || printf 'not installed or inactive\n'
REMOTE
        ;;
    logs)
        ip="$(host_ip)"
        remote "$ip" 'journalctl --user -u vak-gateway.service -n 100 --no-pager'
        ;;
    tunnel)
        ip="$(host_ip)"
        printf 'Open http://127.0.0.1:18901/app while this command runs.\n'
        ssh -N -i "$SSH_KEY" -o BatchMode=yes -o StrictHostKeyChecking=yes \
            -L 127.0.0.1:18901:127.0.0.1:8901 "$SSH_USER@$ip"
        ;;
    copy-token)
        command -v pbcopy >/dev/null || { printf 'copy-token requires macOS pbcopy.\n' >&2; exit 1; }
        ip="$(host_ip)"
        # The CLI's loopback URL percent-encodes the pinned token. Only its
        # decoded token enters the clipboard; nothing secret is printed.
        remote "$ip" '"$HOME/.local/share/vak/local/release/bin/vak" open app --print -C "$HOME/vak-home"' \
            | python3 -c 'import sys, urllib.parse; s=sys.stdin.read().strip(); p=urllib.parse.urlsplit(s); q=urllib.parse.parse_qs(p.query); t=q.get("token",[""])[0]; sys.exit("No pinned gateway token is available") if not t else sys.stdout.write(t)' \
            | pbcopy
        printf 'Bootstrap token copied to clipboard. Use it only for first-owner passkey enrollment.\n'
        ;;
    proxy)
        require_host
        command -v curl >/dev/null || { printf 'curl is required.\n' >&2; exit 1; }
        command -v shasum >/dev/null || { printf 'shasum is required.\n' >&2; exit 1; }
        version=2.11.4
        ip="$(host_ip)"
        reuse=no
        installed_version="$(remote "$ip" '/usr/local/bin/caddy version 2>/dev/null | head -n 1 | cut -d" " -f1' || true)"
        if [[ "$installed_version" == "v$version" ]]; then reuse=yes; fi
        archive="caddy_${version}_linux_arm64.tar.gz"
        temp="$(mktemp -d "${TMPDIR:-/tmp}/vak-caddy.XXXXXXXX")"
        trap 'rm -rf -- "$temp"' EXIT
        if [[ "$reuse" == no ]]; then
            base="https://github.com/caddyserver/caddy/releases/download/v${version}"
            curl -fL --retry 3 "$base/$archive" -o "$temp/$archive"
            curl -fL --retry 3 "$base/caddy_${version}_checksums.txt" -o "$temp/checksums.txt"
            (cd "$temp" && grep -F "  $archive" checksums.txt | shasum -a 512 -c -)
            tar -xzf "$temp/$archive" -C "$temp" caddy
        fi
        cat > "$temp/Caddyfile" <<CADDY
$PUBLIC_HOST {
    encode zstd gzip
    redir / /app 302
    reverse_proxy 127.0.0.1:8901 {
        header_up X-Real-IP {remote_host}
    }
}
CADDY
        cp "$ROOT/scripts/hosting/caddy.service" "$temp/caddy.service"
        stage="$(remote "$ip" 'mktemp -d "$HOME/.vak-proxy.XXXXXXXX"')"
        [[ "$stage" =~ ^/[^[:space:]]+\.vak-proxy\.[A-Za-z0-9]+$ ]] || {
            printf 'Remote staging path was unexpected.\n' >&2; exit 1;
        }
        files=(Caddyfile caddy.service)
        if [[ "$reuse" == no ]]; then files+=(caddy); fi
        for name in "${files[@]}"; do copy_to "$ip" "$temp/$name" "$stage/$name"; done
        remote "$ip" "bash -s -- $(printf '%q' "$stage") $(printf '%q' "$reuse")" <<'REMOTE'
set -Eeuo pipefail
stage="$1"
reuse="$2"
trap 'rm -rf -- "$stage"' EXIT
sudo id caddy >/dev/null 2>&1 || sudo useradd --system --home-dir /var/lib/caddy --create-home --shell /sbin/nologin caddy
sudo install -d -m 0755 /etc/caddy
sudo install -d -o caddy -g caddy -m 0750 /var/lib/caddy
if [[ "$reuse" == no ]]; then sudo install -m 0755 "$stage/caddy" /usr/local/bin/caddy; fi
sudo install -m 0644 "$stage/Caddyfile" /etc/caddy/Caddyfile
sudo install -m 0644 "$stage/caddy.service" /etc/systemd/system/caddy.service
sudo /usr/local/bin/caddy validate --config /etc/caddy/Caddyfile
sudo systemctl daemon-reload
sudo systemctl enable --now caddy.service
sudo systemctl reload caddy.service
systemctl is-active caddy.service
REMOTE
        printf 'Caddy is active. Configure the same HTTPS address in Vakyartha Admin and restart the gateway.\n'
        ;;
    web-check)
        require_host
        for path in /app /admin; do
            curl --fail --silent --show-error --location --output /dev/null \
                --write-out "%{http_code} TLS=%{ssl_verify_result} $path\n" \
                "https://$PUBLIC_HOST$path"
        done
        ;;
    *) usage ;;
esac
