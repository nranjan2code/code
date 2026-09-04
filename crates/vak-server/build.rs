//! Tells Cargo the embedded admin SPA is a build input, and refuses to
//! compile against a bundle that no longer matches its own source.
//!
//! `src/admin_ui.rs` embeds `crates/vak-admin-ui/dist` at compile time via
//! `include_dir!`. That macro reads the directory when it expands, but
//! Cargo's own change detection only recompiles a crate when a file it
//! already knows is a dependency changes — a `.rs` source file, unless a
//! build script says otherwise. Nothing here previously told it dist/ was
//! one, so a `cargo build` after `npm run build` regenerated dist/ could
//! (and did) silently keep the previous incremental build of vak-server,
//! embedding assets that no longer matched `index.html`'s own references
//! to them. The result was a 404 on the JS bundle and the CSS: the admin
//! console loaded an empty shell with nothing in the console to explain
//! why, because no JavaScript ever ran to produce an error.
//!
//! `cargo:rerun-if-changed` on a directory is recursive: any file added,
//! removed, or modified under it invalidates the crate's build cache.
//!
//! Watching dist/ alone left the other half of the same trap open. dist/ is
//! *committed*, so editing `vak-admin-ui/src` and running `cargo build`
//! without `npm run build` is a no-op as far as Cargo is concerned: the
//! binary serves the previous bundle, the browser shows stale UI, and
//! nothing anywhere says so. `scripts/release.sh` catches that before a
//! release ships; the manifest check below catches it on the very next
//! build, which is where it is cheap to fix.

include!("../../scripts/ui_bundle_check.rs");

const ADMIN_UI: &str = "../vak-admin-ui";
/// The workspace client (docs/design/48-web-client.md). This crate embeds
/// its `dist-web/` build under `/app`; `vak-desktop` ships the `dist/` one.
const CLIENT_UI: &str = "../vak-client-ui";
/// The public site at `/` (docs/design/48-web-client.md §4.6). Not an npm
/// bundle, but exactly the same trap: `site/dist` is committed and embedded
/// with `include_dir!`, so editing `site/src` and running `cargo build`
/// without re-running the builder ships the previous pages with nothing
/// anywhere to say so.
const SITE: &str = "site";

fn main() {
    for (ui, dist, hint) in [
        (
            ADMIN_UI,
            "dist",
            "Run `npm run build` in crates/vak-admin-ui and commit dist/ (see docs/design/32-release-engineering.md).",
        ),
        (
            CLIENT_UI,
            "dist-web",
            "Run `npm run build:web` in crates/vak-client-ui.",
        ),
        (
            SITE,
            "dist",
            "Run `python3 crates/vak-server/site/build.py` and commit site/dist/.",
        ),
    ] {
        println!("cargo:rerun-if-changed={ui}/{dist}");
        // The source side must be watched too, or a `src` edit alone never
        // re-runs this script and the check silently stops applying.
        println!("cargo:rerun-if-changed={ui}/src");
        println!("cargo:rerun-if-changed={ui}/index.html");

        if let Err(problem) = check_bundle_matches_source(Path::new(ui), dist) {
            // `site` lives inside this crate; the two UI trees are siblings.
            let crate_name = match ui.strip_prefix("../") {
                Some(sibling) => sibling.to_string(),
                None => format!("vak-server/{ui}"),
            };
            // `cargo::error` is the supported way for a build script to fail
            // the build with a readable message; `panic!` would bury it
            // under a backtrace the reader does not need.
            println!("cargo::error=crates/{crate_name}/{problem}");
            println!("cargo::error={hint}");
            std::process::exit(1);
        }
    }
}
