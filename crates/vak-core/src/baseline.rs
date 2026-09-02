//! The supported baseline, and the one message that refuses anything older.
//!
//! `AGENTS.md` invariant 29: state written before [`BASELINE`] is refused,
//! never partially read and never migrated. Every caller that can tell it is
//! looking at pre-baseline state renders [`refusal`] rather than inventing
//! its own wording — one message means an operator learns the remedy once.

/// The oldest version whose on-disk state this build will accept.
pub const BASELINE: &str = "2.0.0";

/// True when `version` predates [`BASELINE`]. An unparseable version is
/// treated as pre-baseline: every build from the baseline onward stamps a
/// semver string, so anything else was written by something older.
pub fn is_pre_baseline(version: &str) -> bool {
    let Ok(found) = semver::Version::parse(version.trim().trim_start_matches('v')) else {
        return true;
    };
    let Ok(baseline) = semver::Version::parse(BASELINE) else {
        return false;
    };
    found < baseline
}

/// The refusal an operator reads. `what` names the artifact ("this data",
/// "the install manifest") so the sentence stays specific without each
/// caller rewriting the remedy.
pub fn refusal(what: &str, found_version: &str) -> String {
    let found = found_version.trim();
    let found = if found.is_empty() {
        "an unknown version"
    } else {
        found
    };
    format!(
        "{what} was written by vak {found}, which predates the {BASELINE} baseline.\n\
         Versions before {BASELINE} are not supported and cannot be upgraded in place.\n\
         \n  \
         vak self uninstall --purge     then install {BASELINE} and run setup\n\
         \n\
         Your project files are untouched; only vak's own state is removed."
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn versions_below_the_baseline_are_refused() {
        assert!(is_pre_baseline("1.0.3"));
        assert!(is_pre_baseline("0.11.51"));
        assert!(is_pre_baseline("v1.9.9"));
    }

    #[test]
    fn the_baseline_and_anything_above_it_is_accepted() {
        assert!(!is_pre_baseline(BASELINE));
        assert!(!is_pre_baseline("2.0.1"));
        assert!(!is_pre_baseline("3.0.0"));
    }

    #[test]
    fn an_unreadable_version_is_pre_baseline_not_trusted() {
        // Every build from the baseline on stamps semver, so a stamp we
        // cannot parse was written by something older -- failing closed
        // here is what keeps a corrupt or ancient file from being read.
        assert!(is_pre_baseline(""));
        assert!(is_pre_baseline("not-a-version"));
    }

    #[test]
    fn the_refusal_names_the_artifact_the_version_and_the_remedy() {
        let msg = refusal("The install manifest", "1.0.3");
        assert!(msg.contains("The install manifest"));
        assert!(msg.contains("1.0.3"));
        assert!(msg.contains(BASELINE));
        assert!(msg.contains("vak self uninstall --purge"));
        assert!(msg.contains("Your project files are untouched"));
    }
}
