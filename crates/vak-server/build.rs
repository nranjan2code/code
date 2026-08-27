//! Tells Cargo the embedded admin SPA is a build input.
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

fn main() {
    println!("cargo:rerun-if-changed=../vak-admin-ui/dist");
}
