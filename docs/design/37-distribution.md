Status: proposal
(does not describe shipped behavior; paths below are targets rather than
citations and are skipped by check_doc_paths.py)

Distribution Architecture
=========================

## Goal

vak is distributed as a single static binary built from Cargo. This document
describes the target distribution model: signed release artifacts with SBOM
metadata, container images, and a package-manager bridge.

## Target Shapes

| Artifact | Build source | Notes |
|---|---|---|
| `vak` binary | `crates/vak` | Single static binary, musl-linked for Linux |
| `vak-server` binary | `crates/vak-server` | Same core, includes the admin console |
| `vak-desktop` AppImage | `crates/vak-desktop` | Tauri bundle, embedded renderer |
| Container | `Dockerfile` | Alpine + dynamic binary; no Node runtime |

## Release Feed

`vak self update` queries a JSON feed at a pinned URL. The feed contains:

```json
{
  "version": "3.0.24",
  "assets": {
    "linux-x86_64": "sha256:...",
    "macos-aarch64": "sha256:..."
  },
  "signature": "ed25519:..."
}
```

The feed never offers a version below the baseline (rule 29: 2.0.0 is the
supported baseline).

## Package Managers

| Manager | Status |
|---|---|
| Homebrew | `vak` formula in `homebrew-tap` |
| Cargo | `cargo install vak` |
| Nix | `vak` in `nixpkgs` |
| DEB/RPM | Built via CI release pipeline |

## SBOM & Signing

TODO: Integrate `cargo-sbom` and `cosign`-style signing. This is a parked
item (rule 29: release supply-chain hardening is parked).
