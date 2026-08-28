//! The release feed `self update` reads, and the version decision it drives.
//!
//! Two properties this module exists to guarantee:
//!
//! * **Ordering is semantic, not lexical.** Comparing version strings with
//!   `<=` puts `0.10.0` below `0.8.0`, so the first release past `0.9`
//!   reports "up to date" forever. Ordering goes through `semver::Version`.
//! * **Every artifact carries a digest.** A feed without one cannot be
//!   verified after download, so it is rejected rather than trusted.

use std::collections::BTreeMap;

use semver::Version;

/// Feed schema this build writes and understands.
pub const SCHEMA: u32 = 2;

/// One downloadable component of a release.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    pub url: String,
    /// Lowercase hex SHA-256 of the bytes at `url`.
    pub sha256: String,
    #[serde(default)]
    pub required: bool,
}

/// The set of artifacts for one platform.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Platform {
    #[serde(default)]
    pub components: Vec<Artifact>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct Feed {
    #[serde(default)]
    pub schema: u32,
    pub version: String,
    #[serde(default)]
    pub released_at: String,
    #[serde(default)]
    pub notes_url: Option<String>,
    #[serde(default)]
    pub platforms: BTreeMap<String, Platform>,
}

/// Why an update is or is not going to happen.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// The feed offers a strictly newer version.
    Upgrade { from: Version, to: Version },
    /// Installed version is greater than or equal to the feed's.
    UpToDate {
        installed: Version,
        offered: Version,
    },
}

/// Platform key used in the feed, e.g. `macos/aarch64`.
pub fn platform_key() -> String {
    format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Parse a version, tolerating a leading `v` as written in git tags.
pub fn parse_version(raw: &str) -> Result<Version, String> {
    let cleaned = raw.trim().trim_start_matches(['v', 'V']);
    Version::parse(cleaned).map_err(|e| format!("version {raw:?} is not valid semver: {e}"))
}

/// The comparable form of a version.
///
/// `semver::Version` orders build metadata as significant — it reports
/// `0.8.0+build.7 > 0.8.0` — but semver §10 requires build metadata to be
/// ignored when determining precedence. Left alone, a feed that stamps a
/// build suffix would offer a perpetual "upgrade" to the version already
/// installed. Precedence is therefore taken over `(major, minor, patch,
/// pre)` with build metadata discarded.
fn precedence(v: &Version) -> Version {
    Version {
        major: v.major,
        minor: v.minor,
        patch: v.patch,
        pre: v.pre.clone(),
        build: semver::BuildMetadata::EMPTY,
    }
}

impl Feed {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let feed: Feed = serde_json::from_slice(bytes)
            .map_err(|e| format!("release feed is unreadable: {e}"))?;
        if feed.schema > SCHEMA {
            return Err(format!(
                "release feed schema {} is newer than this build understands ({SCHEMA}) — update manually",
                feed.schema
            ));
        }
        Ok(feed)
    }

    pub fn version(&self) -> Result<Version, String> {
        parse_version(&self.version)
    }

    /// Decide whether `installed` should move to this feed's version.
    /// Build metadata is ignored for ordering, per semver; prereleases
    /// order below their release, so `1.0.0-rc.1` never displaces `1.0.0`.
    pub fn decide(&self, installed: &str) -> Result<Decision, String> {
        let offered = self.version()?;
        let installed = parse_version(installed)?;
        if precedence(&offered) > precedence(&installed) {
            Ok(Decision::Upgrade {
                from: installed,
                to: offered,
            })
        } else {
            Ok(Decision::UpToDate { installed, offered })
        }
    }

    /// Artifacts for this platform, rejecting any entry that cannot be
    /// integrity-checked after download.
    pub fn artifacts_for(&self, key: &str) -> Result<Vec<Artifact>, String> {
        let platform = self.platforms.get(key).ok_or_else(|| {
            let known: Vec<&str> = self.platforms.keys().map(String::as_str).collect();
            format!(
                "release {} has no artifacts for {key} (feed lists: {})",
                self.version,
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            )
        })?;
        if platform.components.is_empty() {
            return Err(format!(
                "release {} lists {key} with no components",
                self.version
            ));
        }
        if let Some(bad) = platform
            .components
            .iter()
            .find(|a| a.sha256.trim().is_empty())
        {
            return Err(format!(
                "artifact {:?} has no sha256 in the feed — refusing to install an unverifiable binary",
                bad.name
            ));
        }
        Ok(platform.components.clone())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn feed(version: &str) -> Feed {
        Feed {
            schema: SCHEMA,
            version: version.into(),
            released_at: String::new(),
            notes_url: None,
            platforms: BTreeMap::from([(
                "macos/aarch64".to_string(),
                Platform {
                    components: vec![Artifact {
                        name: "vak".into(),
                        url: "https://example.invalid/vak".into(),
                        sha256: "ab".repeat(32),
                        required: true,
                    }],
                },
            )]),
        }
    }

    #[test]
    fn double_digit_minor_is_newer_than_single_digit() {
        // The defect this module exists to prevent: as strings,
        // "0.10.0" <= "0.8.0", which reported every later release as
        // already installed.
        assert!("0.10.0" <= "0.8.0", "string ordering really is this wrong");
        assert_eq!(
            feed("0.10.0").decide("0.8.0").unwrap(),
            Decision::Upgrade {
                from: Version::parse("0.8.0").unwrap(),
                to: Version::parse("0.10.0").unwrap(),
            }
        );
    }

    #[test]
    fn equal_and_older_feeds_do_not_trigger_an_update() {
        assert!(matches!(
            feed("0.8.0").decide("0.8.0").unwrap(),
            Decision::UpToDate { .. }
        ));
        assert!(matches!(
            feed("0.7.9").decide("0.8.0").unwrap(),
            Decision::UpToDate { .. }
        ));
    }

    #[test]
    fn a_prerelease_never_displaces_the_matching_release() {
        assert!(matches!(
            feed("1.0.0-rc.1").decide("1.0.0").unwrap(),
            Decision::UpToDate { .. }
        ));
        assert!(matches!(
            feed("1.0.0").decide("1.0.0-rc.1").unwrap(),
            Decision::Upgrade { .. }
        ));
        assert!(matches!(
            feed("1.0.0-rc.2").decide("1.0.0-rc.1").unwrap(),
            Decision::Upgrade { .. }
        ));
    }

    #[test]
    fn build_metadata_is_accepted_and_ignored_for_ordering() {
        // semver::Version's own Ord disagrees with semver spec 10 here:
        // it reports 0.8.0+build.7 as greater than 0.8.0. Untreated, a
        // feed stamping a build suffix offers an endless upgrade to the
        // version already installed.
        assert!(
            Version::parse("0.8.0+build.7").unwrap() > Version::parse("0.8.0").unwrap(),
            "the library really does order build metadata"
        );
        assert!(matches!(
            feed("0.8.0+build.7").decide("0.8.0").unwrap(),
            Decision::UpToDate { .. }
        ));
        assert!(matches!(
            feed("0.8.0").decide("0.8.0+build.7").unwrap(),
            Decision::UpToDate { .. }
        ));
        // A real version bump still wins through the build suffix.
        assert!(matches!(
            feed("0.8.1+build.1").decide("0.8.0+build.9").unwrap(),
            Decision::Upgrade { .. }
        ));
    }

    #[test]
    fn leading_v_from_a_git_tag_is_tolerated() {
        assert_eq!(
            parse_version("v1.2.3").unwrap(),
            Version::parse("1.2.3").unwrap()
        );
    }

    #[test]
    fn an_artifact_without_a_digest_is_refused() {
        let mut f = feed("0.9.0");
        f.platforms.get_mut("macos/aarch64").unwrap().components[0].sha256 = "  ".into();
        let err = f.artifacts_for("macos/aarch64").unwrap_err();
        assert!(err.contains("unverifiable"), "got: {err}");
    }

    #[test]
    fn a_missing_platform_names_what_the_feed_does_offer() {
        let err = feed("0.9.0").artifacts_for("linux/x86_64").unwrap_err();
        assert!(err.contains("macos/aarch64"), "got: {err}");
    }

    #[test]
    fn a_future_schema_is_refused_rather_than_misread() {
        let raw = br#"{"schema":99,"version":"9.9.9","platforms":{}}"#;
        assert!(
            Feed::parse(raw)
                .unwrap_err()
                .contains("newer than this build")
        );
    }
}
