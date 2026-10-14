//! The supported baseline, and the one message that refuses anything older.
//!
//! `AGENTS.md` invariant 29: state written before [`BASELINE`] is refused,
//! never partially read and never migrated. Every caller that can tell it is
//! looking at pre-baseline state renders [`refusal`] rather than inventing
//! its own wording — one message means an operator learns the remedy once.

/// The oldest version whose on-disk state this build will accept.
pub const BASELINE: &str = "8.0.0";

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
    // A prerelease of the baseline (`7.0.0-dev`) is the baseline line itself,
    // so compare the release numbers, not semver's prerelease ordering.
    (found.major, found.minor, found.patch) < (baseline.major, baseline.minor, baseline.patch)
}

/// Refuses a data home written before the baseline. A home names the
/// version that created it in its [`HOME_VERSION_FILE`]; one that holds
/// records (a tenant's object store, or the session and Agent trees older
/// homes kept at the top) but no such file was made before the file
/// existed, so it is older than any baseline that reads it. A home holding
/// no records passes, whatever bound its folders, and nothing is written:
/// [`stamp_data_home`] is the one writer.
///
/// [`HOME_VERSION_FILE`]: vak_config::paths::HOME_VERSION_FILE
pub fn check_data_home(data: &std::path::Path) -> Result<(), String> {
    let what = format!("The data home {}", data.display());
    match std::fs::read_to_string(data.join(vak_config::paths::HOME_VERSION_FILE)) {
        Ok(version) if is_pre_baseline(&version) => Err(refusal(&what, &version)),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if holds_records(data) {
                Err(refusal(&what, ""))
            } else {
                Ok(())
            }
        }
        Err(error) => Err(format!("{what} cannot be read: {}", error.kind())),
    }
}

/// Whether `data` holds records: a tenant's object store, or the session
/// tree a home before the tenant layout kept at its top. An Agent's folder
/// (`agents/<id>`) is not a record: current paths make it before any Core
/// has stamped the home, and counting it refused a new home for good.
fn holds_records(data: &std::path::Path) -> bool {
    let top = data.join("sessions").exists();
    let stores = std::fs::read_dir(data.join(vak_config::paths::TENANTS_DIR))
        .map(|tenants| {
            tenants.flatten().any(|tenant| {
                tenant
                    .path()
                    .join(vak_config::paths::TENANT_STORE_DIR)
                    .exists()
            })
        })
        .unwrap_or(false);
    top || stores
}

/// Names this build as the creator of a data home that has no
/// [`HOME_VERSION_FILE`] yet. Call it only after [`check_data_home`]
/// passed, which means the home is empty; an existing file is never
/// rewritten.
///
/// [`HOME_VERSION_FILE`]: vak_config::paths::HOME_VERSION_FILE
pub fn stamp_data_home(data: &std::path::Path) -> std::io::Result<()> {
    let path = data.join(vak_config::paths::HOME_VERSION_FILE);
    if path.exists() {
        return Ok(());
    }
    vak_session::fence::check().map_err(std::io::Error::other)?;
    std::fs::create_dir_all(data)?;
    // Staged under a name of its own and linked into place, so two
    // processes opening an empty home at once never share a staging file
    // and the first to link is the one that named it.
    let staged = data.join(format!(
        ".{}.{}",
        vak_config::paths::HOME_VERSION_FILE,
        uuid::Uuid::now_v7()
    ));
    std::fs::write(&staged, crate::APP_VERSION)?;
    let linked = std::fs::hard_link(&staged, &path);
    let _ = std::fs::remove_file(&staged);
    match linked {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => Err(error),
        _ => Ok(()),
    }
}

/// The refusal an operator reads. `what` names the artifact ("this data",
/// "the install manifest") so the sentence stays specific without each
/// caller rewriting the remedy.
pub fn refusal(what: &str, found_version: &str) -> String {
    let found = found_version.trim();
    let found = if found.is_empty() {
        "a version of vak that did not record itself".to_string()
    } else {
        format!("vak {found}")
    };
    format!(
        "{what} was written by {found}, which predates the {BASELINE} baseline.\n\
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
        assert!(is_pre_baseline("6.0.0"));
        assert!(is_pre_baseline("7.0.0"));
        assert!(is_pre_baseline("7.3.0"));
    }

    #[test]
    fn the_baseline_and_anything_above_it_is_accepted() {
        assert!(!is_pre_baseline(BASELINE));
        assert!(!is_pre_baseline("8.0.0-dev"));
        assert!(!is_pre_baseline("8.0.1"));
        assert!(!is_pre_baseline("9.0.0"));
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

    #[test]
    fn a_home_without_its_version_is_refused_with_one_message() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_data_home(dir.path()).is_ok(), "a fresh home is fine");
        let store = format!(
            "{}/local/{}",
            vak_config::paths::TENANTS_DIR,
            vak_config::paths::TENANT_STORE_DIR
        );
        for state in ["sessions", store.as_str()] {
            let home = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(home.path().join(state)).unwrap();
            let refused = check_data_home(home.path()).unwrap_err();
            assert!(refused.contains("predates the 8.0.0 baseline"), "{refused}");
            assert!(refused.contains("vak self uninstall --purge"), "{refused}");
        }
    }

    #[test]
    fn a_home_names_the_version_that_made_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(vak_config::paths::HOME_VERSION_FILE);
        std::fs::write(&file, "7.3.0").unwrap();
        let refused = check_data_home(dir.path()).unwrap_err();
        assert!(refused.contains("7.3.0"), "{refused}");
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir_all(
            dir.path()
                .join(vak_config::paths::TENANTS_DIR)
                .join("local"),
        )
        .unwrap();
        assert!(
            check_data_home(dir.path()).is_ok(),
            "a bound folder is not a record"
        );
        stamp_data_home(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), crate::APP_VERSION);
        std::fs::create_dir_all(dir.path().join(vak_config::paths::TENANTS_DIR)).unwrap();
        assert!(check_data_home(dir.path()).is_ok());
        std::fs::write(&file, "9.1.0").unwrap();
        stamp_data_home(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "9.1.0",
            "never rewritten"
        );
    }

    #[test]
    fn a_home_with_only_caches_and_logs_is_not_pre_baseline() {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["cache", "logs", "vak-home", "runtime", "agents/vak"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        assert!(check_data_home(dir.path()).is_ok());
    }
}
