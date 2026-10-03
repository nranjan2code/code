//! Current social connector readiness. YouTube allows a bounded owner-only
//! preview; model-facing API calls remain unavailable pending a reviewed
//! broker adapter and provider-compatible retention path.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    Blocked,
    OwnerPreview,
    IdentityLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Connector {
    pub id: &'static str,
    pub platform: &'static str,
    pub summary: &'static str,
    pub readiness: Readiness,
    pub reason: &'static str,
    pub official_api: &'static str,
}

pub const CONNECTORS: [Connector; 4] = [
    Connector {
        id: "social-linkedin",
        platform: "LinkedIn",
        summary: "Owner-visible LinkedIn profile name through OIDC; content API operations remain gated.",
        readiness: Readiness::IdentityLink,
        reason: "An owner-only OpenID Connect profile link is available after native PKCE is enabled for your app. LinkedIn post search and content tools remain unavailable.",
        official_api: "https://learn.microsoft.com/en-us/linkedin/shared/authentication/getting-access",
    },
    Connector {
        id: "social-reddit",
        platform: "Reddit",
        summary: "Owner-only Reddit search preview through the official Data API.",
        readiness: Readiness::OwnerPreview,
        reason: "Owner-only search preview with your own Reddit app Client ID. Results stay on this screen and are never saved or sent to the agent, so Reddit's deleted-content rule has nothing to remove. Agent use stays blocked until Reddit eligibility and erasure are resolved.",
        official_api: "https://redditinc.com/policies/data-api-terms",
    },
    Connector {
        id: "social-x",
        platform: "X",
        summary: "Owner-only X search preview with a hard monthly request ceiling.",
        readiness: Readiness::OwnerPreview,
        reason: "Owner-only search preview with your own bearer token. Every search counts against a monthly ceiling you set, and searching stops at it. Results are not saved or sent to the agent.",
        official_api: "https://developer.x.com/",
    },
    Connector {
        id: "social-youtube",
        platform: "YouTube",
        summary: "Bounded YouTube Data API reads, subject to quota and data refresh rules.",
        readiness: Readiness::OwnerPreview,
        reason: "Owner-only search preview is available. Results stay out of agent history and are not saved by Vakyartha; API quota and YouTube data refresh rules apply.",
        official_api: "https://developers.google.com/youtube/v3",
    },
];

pub fn find(id: &str) -> Option<&'static Connector> {
    CONNECTORS.iter().find(|connector| connector.id == id)
}

/// Embedded declarative package bytes for the built-in guidance add-ons.
/// These files are inert data; plugin installation never runs package code.
pub fn package(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "social-linkedin" => Some((
            include_str!("../../../../packages/social-linkedin/vak-plugin.json"),
            include_str!("../../../../packages/social-linkedin/skills/social-linkedin/SKILL.md"),
        )),
        "social-reddit" => Some((
            include_str!("../../../../packages/social-reddit/vak-plugin.json"),
            include_str!("../../../../packages/social-reddit/skills/social-reddit/SKILL.md"),
        )),
        "social-x" => Some((
            include_str!("../../../../packages/social-x/vak-plugin.json"),
            include_str!("../../../../packages/social-x/skills/social-x/SKILL.md"),
        )),
        "social-youtube" => Some((
            include_str!("../../../../packages/social-youtube/vak-plugin.json"),
            include_str!("../../../../packages/social-youtube/skills/social-youtube/SKILL.md"),
        )),
        _ => None,
    }
}

/// Declarative, platform-specific presentation previews shipped with each
/// add-on. They use the existing research renderer and stay unactivated until
/// an operator chooses a layout in Presentation settings.
pub fn presentations(id: &str) -> Option<[(&'static str, &'static str); 3]> {
    let files = match id {
        "social-reddit" => [
            include_str!("../../../../packages/social-reddit/presentation/research-brief.json"),
            include_str!("../../../../packages/social-reddit/presentation/source-review.json"),
            include_str!("../../../../packages/social-reddit/presentation/takeaway-board.json"),
        ],
        "social-youtube" => [
            include_str!("../../../../packages/social-youtube/presentation/research-brief.json"),
            include_str!("../../../../packages/social-youtube/presentation/source-review.json"),
            include_str!("../../../../packages/social-youtube/presentation/takeaway-board.json"),
        ],
        "social-x" => [
            include_str!("../../../../packages/social-x/presentation/research-brief.json"),
            include_str!("../../../../packages/social-x/presentation/source-review.json"),
            include_str!("../../../../packages/social-x/presentation/takeaway-board.json"),
        ],
        "social-linkedin" => [
            include_str!("../../../../packages/social-linkedin/presentation/research-brief.json"),
            include_str!("../../../../packages/social-linkedin/presentation/source-review.json"),
            include_str!("../../../../packages/social-linkedin/presentation/takeaway-board.json"),
        ],
        _ => return None,
    };
    Some([
        ("presentation/research-brief.json", files[0]),
        ("presentation/source-review.json", files[1]),
        ("presentation/takeaway-board.json", files[2]),
    ])
}

/// Writes the built-in package for `id` into `dir`, exactly as it is installed.
/// The one layout the seed, the install endpoint and the update check share.
pub fn stage_package(id: &str, dir: &std::path::Path) -> std::io::Result<()> {
    let Some((manifest, skill)) = package(id) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no built-in social package {id}"),
        ));
    };
    std::fs::create_dir_all(dir.join("skills"))?;
    std::fs::write(dir.join("vak-plugin.json"), manifest)?;
    std::fs::write(dir.join("skills/SKILL.md"), skill)?;
    if let Some(files) = presentations(id) {
        std::fs::create_dir_all(dir.join("presentation"))?;
        for (relative, contents) in files {
            std::fs::write(dir.join(relative), contents)?;
        }
    }
    Ok(())
}

/// Digest of the built-in package for `id`, as the plugin store computes it,
/// so a client can tell an installed copy from an older built-in one.
pub fn package_digest(id: &str) -> Option<String> {
    static DIGESTS: std::sync::OnceLock<std::collections::HashMap<&'static str, String>> =
        std::sync::OnceLock::new();
    DIGESTS
        .get_or_init(|| {
            CONNECTORS
                .iter()
                .filter_map(|connector| {
                    let dir = std::env::temp_dir().join(format!(
                        "vak-social-digest-{}-{}",
                        connector.id,
                        uuid::Uuid::now_v7()
                    ));
                    let digest = stage_package(connector.id, &dir)
                        .ok()
                        .and_then(|()| vak_plugin::inspect_package(&dir).ok())
                        .map(|inspection| inspection.digest);
                    let _ = std::fs::remove_dir_all(&dir);
                    digest.map(|digest| (connector.id, digest))
                })
                .collect()
        })
        .get(id)
        .cloned()
}
