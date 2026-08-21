#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_tools::ResourceClaims;

fn claims(paths: &[&str]) -> ResourceClaims {
    ResourceClaims {
        exclusive: false,
        read_only: false,
        paths: paths.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn unclaimed_never_conflicts() {
    let a = ResourceClaims::default();
    let b = claims(&["src/**"]);
    assert!(!a.conflicts(&b));
    assert!(!b.conflicts(&a));
}

#[test]
fn read_only_never_conflicts() {
    let a = ResourceClaims {
        exclusive: false,
        read_only: true,
        paths: Vec::new(),
    };
    let b = claims(&["src/**"]);
    assert!(!a.conflicts(&b));
    assert!(!b.conflicts(&a));
}

#[test]
fn exclusive_conflicts_with_any_claimed() {
    let a = ResourceClaims {
        exclusive: true,
        read_only: false,
        paths: Vec::new(),
    };
    let b = claims(&["docs/**"]);
    assert!(a.conflicts(&b));
    assert!(b.conflicts(&a));
}

#[test]
fn overlapping_prefixes_conflict() {
    assert!(claims(&["src/**"]).conflicts(&claims(&["src/auth/**"])));
    assert!(claims(&["src/auth/**"]).conflicts(&claims(&["src/**"])));
    assert!(claims(&["src/auth/**"]).conflicts(&claims(&["src/auth/**"])));
}

#[test]
fn sibling_scopes_are_disjoint() {
    assert!(!claims(&["src/auth/**"]).conflicts(&claims(&["src/billing/**"])));
    assert!(!claims(&["src/auth/**"]).conflicts(&claims(&["docs/**"])));
}

#[test]
fn normalization_handles_globs_and_slashes() {
    // "src/" vs "src/**" must be seen as the same scope
    assert!(claims(&["src/"]).conflicts(&claims(&["src/**"])));
    assert!(claims(["src"].as_slice()).conflicts(&claims(&["src/*"])));
}
