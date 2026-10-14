//! The packages shipped inside Vakyartha: the `packages/` folder, embedded so
//! setup works from an installed binary, and installed into the Shared layer
//! by `seed::seed_shared_capabilities`. A shipped package is an ordinary
//! plugin package; nothing in the harness names what is inside one.

use include_dir::{Dir, include_dir};

static PACKAGES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../packages");

/// A package Vakyartha ships, and whether setup installs it enabled. One
/// that reaches an outside service starts disabled until the owner turns it
/// on.
pub struct Shipped {
    pub name: &'static str,
    pub enabled: bool,
}

pub const SHIPPED: &[Shipped] = &[
    Shipped {
        name: "developer-starter",
        enabled: true,
    },
    Shipped {
        name: "everyday-starter",
        enabled: true,
    },
    Shipped {
        name: "answer-layouts",
        enabled: true,
    },
    Shipped {
        name: "social-linkedin",
        enabled: false,
    },
    Shipped {
        name: "social-reddit",
        enabled: false,
    },
    Shipped {
        name: "social-x",
        enabled: false,
    },
    Shipped {
        name: "social-youtube",
        enabled: false,
    },
];

pub fn shipped(name: &str) -> Option<&'static Shipped> {
    SHIPPED.iter().find(|package| package.name == name)
}

/// Writes the shipped package `name` into `dir`, exactly as it is in
/// `packages/<name>`.
pub fn stage(name: &str, dir: &std::path::Path) -> std::io::Result<()> {
    let package = shipped(name)
        .and_then(|_| PACKAGES.get_dir(name))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no shipped package {name}"),
            )
        })?;
    write_dir(package, name, dir)
}

fn write_dir(source: &Dir<'_>, prefix: &str, dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for file in source.files() {
        let relative = file.path().strip_prefix(prefix).unwrap_or(file.path());
        let target = dir.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, file.contents())?;
    }
    for child in source.dirs() {
        write_dir(child, prefix, dir)?;
    }
    Ok(())
}

/// The digest of the shipped package `name` as the plugin store computes
/// it, so a client can tell an installed copy from the shipped one.
pub fn digest(name: &str) -> Option<String> {
    static DIGESTS: std::sync::OnceLock<std::collections::HashMap<&'static str, String>> =
        std::sync::OnceLock::new();
    DIGESTS
        .get_or_init(|| {
            SHIPPED
                .iter()
                .filter_map(|package| {
                    let dir = std::env::temp_dir().join(format!(
                        "vak-shipped-{}-{}",
                        package.name,
                        uuid::Uuid::now_v7()
                    ));
                    let digest = stage(package.name, &dir)
                        .ok()
                        .and_then(|()| vak_plugin::inspect_package(&dir).ok())
                        .map(|inspection| inspection.digest);
                    let _ = std::fs::remove_dir_all(&dir);
                    digest.map(|digest| (package.name, digest))
                })
                .collect()
        })
        .get(name)
        .cloned()
}
