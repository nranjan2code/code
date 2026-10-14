//! The packages shipped inside Vakyartha are embedded from `packages/`
//! (`src/packages.rs`); a change to any file there rebuilds this crate.
//! `rerun-if-changed` on a directory is recursive. A build script speaks
//! to cargo on stdout, which is what the println! below is for.
#![allow(clippy::disallowed_macros)]

fn main() {
    println!("cargo:rerun-if-changed=../../packages");
}
