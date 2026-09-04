# The headless Linux image: `vak serve`, the web client at /app, and the
# operations console at /admin — one binary, one port
# (docs/design/48-web-client.md, docs/hosting.md).
#
# NO NODE IN EITHER STAGE. That is a design claim being tested, not an
# omission: `crates/vak-admin-ui/dist` and `crates/vak-client-ui/dist-web`
# are committed precisely so a headless box builds the server without a
# JavaScript toolchain. If this image ever needs npm, that design has
# regressed and the fix belongs in the bundles, not here.
#
# The desktop shell is deliberately absent — it needs a display, and its
# bundle (`crates/vak-client-ui/dist`) is gitignored.

# ---------------------------------------------------------------- builder
FROM rust:1-bookworm AS builder

# `rusqlite` is vendored ("bundled") and `ring` compiles C, so a toolchain
# is required; TLS is rustls, so OpenSSL headers are NOT.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        perl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Dependency layer: manifests first, so a source-only edit does not
# re-download and rebuild the whole dependency graph.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY scripts ./scripts

# The commit this image was built from, so `--version`, `/version` and
# `install.json` can name it. `docker/build.sh` derives and passes it;
# "unknown" is honest when absent, and is what a bare `docker build` gets.
ARG VAK_GIT_SHA=unknown
ENV VAK_GIT_SHA=${VAK_GIT_SHA}

# Only what a headless host runs. `--locked` so the image cannot silently
# resolve different dependency versions than the checkout was tested with.
RUN cargo build --release --locked \
        --package vak \
        --package vak-delivery

# ---------------------------------------------------------------- runtime
FROM debian:bookworm-slim AS runtime

# ca-certificates: provider APIs are HTTPS.
# git:             worktrees and checkpoints are git operations, not optional.
# bash:            the `bash` tool runs real commands.
# curl:            the healthcheck below, and nothing else.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
        bash \
        curl \
    && rm -rf /var/lib/apt/lists/*

# An agent that can run commands should not be running them as root.
RUN useradd --create-home --shell /bin/bash --uid 10001 vak \
    && mkdir -p /workspace /home/vak/.local/share /home/vak/.cache /home/vak/.local/state \
    && chown -R vak:vak /workspace /home/vak

# Binaries stay ROOT-owned and world-executable, deliberately. Handing them
# to the `vak` user would let a compromised agent process rewrite its own
# executable — the one file that must outlive any tool call it makes.
COPY --from=builder /src/target/release/vak /usr/local/bin/vak
COPY --from=builder \
     /src/target/release/vak-delivery-worker /usr/local/bin/vak-delivery-worker
RUN chmod 0755 /usr/local/bin/vak /usr/local/bin/vak-delivery-worker

USER vak

# XDG data home (docs/design/32 canonical layout): sessions, memory, tasks,
# config, and the audit logs all live here. Mount it to keep them.
ENV XDG_DATA_HOME=/home/vak/.local/share \
    XDG_CACHE_HOME=/home/vak/.cache \
    XDG_STATE_HOME=/home/vak/.local/state
VOLUME ["/home/vak/.local/share", "/workspace"]

# The workspace the agent acts on. Bind-mount your project here.
WORKDIR /workspace

EXPOSE 8901

# Loopback inside the container is not reachable from outside it, so the
# image binds all interfaces — and `[server] trusted_hosts` is what governs
# who may actually talk to it (invariant 34). The container boundary IS the
# firewall here; publish the port deliberately.
ENV VAK_CONTAINER=1

# Probes the server, not the configuration. `vak doctor` exits non-zero for
# an unset provider key — a legitimate state for a container that has not
# been given one yet — which would report a perfectly healthy server as
# unhealthy forever. `/health` answers the question actually being asked:
# is it serving?
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8901/health || exit 1

ENTRYPOINT ["/usr/local/bin/vak"]
CMD ["serve", "--host", "0.0.0.0", "--port", "8901"]
