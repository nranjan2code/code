//! Tauri codegen, plus the same stale-bundle guard `vak-server` uses.
//!
//! `vak-desktop/ui/dist` is gitignored rather than committed, so it can
//! never be *shipped* stale the way the admin bundle once was — but a local
//! `cargo build` after a UI edit still produces an app serving the previous
//! frontend, silently. Same trap, same check.

include!("../../scripts/ui_bundle_check.rs");

const UI: &str = "ui";

fn main() {
    println!("cargo:rerun-if-changed={UI}/src");
    println!("cargo:rerun-if-changed={UI}/index.html");

    if let Err(problem) = check_bundle_matches_source(Path::new(UI)) {
        let problem = format!("crates/vak-desktop/ui/{problem}");
        // `cargo::error` is the supported way for a build script to fail the
        // build with a readable message; `panic!` would bury it under a
        // backtrace the reader does not need.
        println!("cargo::error={problem}");
        println!("cargo::error=Run `npm run build` in crates/vak-desktop/ui.");
        std::process::exit(1);
    }

    tauri_build::build()
}
