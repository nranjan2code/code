// A build script talks to cargo on stdout.
#![allow(clippy::disallowed_macros)]
//! Keeps the build stamp honest, and composes the version string.
//!
//! Two jobs, both about the same fact — which commit this binary is.
//!
//! 1. `VAK_GIT_SHA` is read with `option_env!`, which is baked in at COMPILE
//!    time. Cargo does not know an arbitrary environment variable is a build
//!    input, so without `rerun-if-env-changed` it happily reuses a cached
//!    object compiled against an older value, and `install.json` then records
//!    the commit of whenever that object was last built. A manifest naming
//!    the wrong commit is worse than one saying nothing: it is what an
//!    operator reads to answer "what is deployed here", and a confidently
//!    stale answer sends them debugging code that was never running.
//!
//! 2. `VAK_VERSION` is the string `--version` prints. Composing it here
//!    rather than in the CLI because `concat!` takes only literals, so
//!    "release, plus a sha if there is one" cannot be expressed inline.
//!
//! `scripts/build.sh` and `scripts/release.sh` set the sha; a plain
//! `cargo build` leaves it unset and both the manifest and `--version`
//! honestly report just the release.

fn main() {
    println!("cargo:rerun-if-env-changed=VAK_GIT_SHA");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let stamped = match std::env::var("VAK_GIT_SHA") {
        Ok(sha) if !sha.trim().is_empty() && sha != "unknown" => {
            format!("{version} ({})", sha.trim())
        }
        _ => version,
    };
    println!("cargo:rustc-env=VAK_VERSION={stamped}");
}
