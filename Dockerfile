# Headless Linux image for the vak server (docs/design/28-operations.md).
#
# Ships the `vak` CLI — which serves the gateway over HTTP+SSE via
# `vak serve` — and the `vak-delivery-worker`. `vak-desktop` (the Tauri GUI)
# is intentionally NOT built: it is desktop-only and needs a display.
#
# The committed frontend bundles (vak-client-ui/dist-web, vak-admin-ui/dist,
# site/dist) are embedded by vak-server at build time, so this image builds
# with no Node in it at all. .dockerignore keeps target/, secrets, and host
# state out of the build context.

# ---- builder -------------------------------------------------------------
FROM rust:bookworm AS builder
WORKDIR /src
# Copy after .dockerignore is applied; the lockfile pins the same dep graph
# the release gate tested.
COPY . .
# `vak` pulls in vak-server (the HTTP+SSE + embedded bundles). `vak-delivery`
# also produces the `vak-delivery-worker` binary. Neither needs Tauri.
# `--locked` pins the same dependency graph the release gate tested (invariant
# 15); the `VAK_GIT_SHA` ARG stamps the commit into the binary so a running
# container can name its own build, matching the release binary.
ARG VAK_GIT_SHA=unknown
ENV VAK_GIT_SHA=${VAK_GIT_SHA}
RUN cargo build --release --locked --package vak --package vak-delivery

# ---- runtime -------------------------------------------------------------
# version is authoritative in Cargo.toml and baked into the binary
# (see crates/vak/build.rs); `vak --version` is the source of truth.
FROM debian:bookworm-slim AS runtime
LABEL org.opencontainers.image.title="vak" \
      org.opencontainers.image.description="vak agent server (gateway over HTTP+SSE)"

RUN addgroup --system --gid 1111 vak \
 && adduser --system --ingroup vak --uid 1111 \
            --home /var/lib/vak --gecos "" vak

COPY --from=builder /src/target/release/vak                  /usr/local/bin/vak
COPY --from=builder /src/target/release/vak-delivery-worker  /usr/local/bin/vak-delivery-worker

# The gateway's data home (sessions, config, provider keys). Persist it
# across upgrades with a volume:  -v vak-home:/home/vak/.local/share
ENV VAK_HOME=/home/vak/.local/share
WORKDIR /var/lib/vak
USER vak

# Default port is 8901 (see `vak serve --help`). The server binds loopback by
# default; to expose it, supply a config with `[server] bind = "0.0.0.0"` and
# an allow-listed `[server] trusted_hosts` (never expose a shell-capable
# agent without those guards — docs/design/48-web-client.md §4.2).
EXPOSE 8901

# `vak serve` is the default entrypoint; override by passing a subcommand,
# e.g.  docker run --rm vak:3.0.12 --version
ENTRYPOINT ["vak"]
CMD ["serve"]
