//! Learning loop (docs/design/26-learning.md): `remember` appends durable
//! notes, `forget_memory` removes them, and `propose_skill` queues skill drafts.
//! Proposals never enter discovery by themselves — promotion is an explicit
//! human action over HTTP or CLI.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::memory;

const KINDS: [&str; 5] = ["fact", "decision", "preference", "reference", "invariant"];

// ---- remember ---------------------------------------------------------------

pub struct RememberTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
}

/// Remove one curated memory note after `session_search` identifies its
/// stable id. Session ledgers remain append-only; this edits only MEMORY.md.
pub struct ForgetMemoryTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
}

#[async_trait::async_trait]
impl vak_tools::Tool for ForgetMemoryTool {
    fn name(&self) -> &str {
        "forget_memory"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["memory"]
    }

    fn description(&self) -> &str {
        "Forget one saved memory note when the user asks to delete or forget it. First use session_search with the note's topic to identify the exact saved note; pass its returned memory/<id> or profile/<id> source id here. If the result is ambiguous, ask which note they mean. This removes the note from durable memory and future memory search; it does not erase the conversation that originally created it."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "note_id": {"type": "string", "description": "Exact memory/<id> or profile/<id> source id returned by session_search"}
            },
            "required": ["note_id"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(source_id) = args.get("note_id").and_then(Value::as_str) else {
            return vak_tools::ToolOutput::error("missing required argument 'note_id'");
        };
        let (scope, result) = if let Some(id) = source_id.strip_prefix("memory/") {
            (
                "workspace",
                memory::forget_workspace_note(&self.sessions_home, &self.cwd, id),
            )
        } else if let Some(id) = source_id.strip_prefix("profile/") {
            (
                "profile",
                memory::forget_profile_note(&self.sessions_home, id),
            )
        } else {
            return vak_tools::ToolOutput::error(
                "note_id must be the exact memory/<id> or profile/<id> source id from session_search",
            );
        };
        match result {
            Ok(_) => vak_tools::ToolOutput::ok(format!(
                "Forgot the {scope} memory note. It will no longer appear in future memory searches; its source conversation remains in history."
            )),
            Err(error) => {
                vak_tools::ToolOutput::error(format!("could not forget memory note: {error}"))
            }
        }
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: true,
            read_only: false,
            paths: vec![],
        }
    }
}

#[async_trait::async_trait]
impl vak_tools::Tool for RememberTool {
    fn name(&self) -> &str {
        "remember"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["memory"]
    }

    fn description(&self) -> &str {
        "Persist a durable note about this workspace for FUTURE sessions \
         (decisions, facts, preferences, pointers). Use sparingly for things \
         worth remembering after this conversation ends — not transient \
         details. Notes are recalled via session_search and are visible to \
         the user, who can edit or forget them. When asked to forget a note, \
         find its exact source id with session_search, then call forget_memory; \
         do not replace it with a note saying not to remember it."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "note": {"type": "string", "description": "The content to persist"},
                "kind": {"type": "string", "enum": KINDS.to_vec(),
                         "description": "One of fact/decision/preference/reference/invariant (default fact)"},
                "tag":  {"type": "string", "description": "Short slug for grouping, e.g. 'deploy-rollbacks'"}
            },
            "required": ["note"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(note) = args.get("note").and_then(Value::as_str).map(str::trim) else {
            return vak_tools::ToolOutput::error("missing required argument 'note'");
        };
        let kind = args.get("kind").and_then(Value::as_str).unwrap_or("fact");
        if !KINDS.contains(&kind) {
            return vak_tools::ToolOutput::error(format!(
                "unknown kind '{kind}'; expected one of {KINDS:?}"
            ));
        }
        let tag = args.get("tag").and_then(Value::as_str).unwrap_or("");
        match memory::append_note(
            &self.sessions_home,
            &self.cwd,
            kind,
            tag,
            &self.session_id,
            note,
        ) {
            Ok(note) => vak_tools::ToolOutput::ok(format!(
                "remembered ({kind}{tag_suffix}) as memory/{}. It will surface in future session_search queries; use forget_memory with that source id if the user later asks to forget it.",
                note.id,
                tag_suffix = if tag.is_empty() {
                    String::new()
                } else {
                    format!(", tag '{tag}'")
                }
            )),
            Err(e) => vak_tools::ToolOutput::error(format!("could not persist note: {e}")),
        }
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: true,
            read_only: false,
            paths: vec![],
        }
    }
}

// ---- propose_skill ----------------------------------------------------------

pub struct SkillProposal {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    /// The conversation (and turn, when recorded) the draft came from, read
    /// from its `proposed-by` trailer.
    pub derived_from: Option<vak_session::trace::DerivedFrom>,
}

/// Parse `<!-- proposed-by: <sid> at <ts>; ...; turn: <t> -->`.
pub fn proposal_provenance(body: &str) -> Option<vak_session::trace::DerivedFrom> {
    let line = body.lines().rev().find(|l| l.contains("proposed-by:"))?;
    let rest = line.split("proposed-by:").nth(1)?.trim();
    let conversation = rest.split_whitespace().next()?.to_string();
    let turn = line
        .split(';')
        .find_map(|part| part.trim().strip_prefix("turn:"))
        .map(|t| t.trim().trim_end_matches("-->").trim().to_string())
        .filter(|t| !t.is_empty());
    Some(vak_session::trace::DerivedFrom { conversation, turn })
}

fn proposals_dir(scope: &vak_config::scope::AgentScope, cwd: &Path) -> PathBuf {
    scope.skill_proposals(cwd)
}

pub struct ProposeSkillTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
}

#[async_trait::async_trait]
impl vak_tools::Tool for ProposeSkillTool {
    fn name(&self) -> &str {
        "propose_skill"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["memory"]
    }

    fn description(&self) -> &str {
        "Draft a reusable SKILL from something learned this session (a \
         procedure that worked, a gotcha and its fix). Goes to a review \
         queue — it becomes available to future runs ONLY after the user \
         promotes it. Do not propose one-off steps."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string",
                         "description": "kebab-case identifier, e.g. 'rotate-release-tags'"},
                "description": {"type": "string",
                                "description": "One line: what it is for and when to use it"},
                "instructions": {"type": "string",
                                 "description": "Markdown procedure the future agent should follow"}
            },
            "required": ["name", "description", "instructions"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(name) = sanitize_name(args.get("name").and_then(Value::as_str).unwrap_or(""))
        else {
            return vak_tools::ToolOutput::error(
                "'name' must be kebab-case (lowercase letters, digits, dashes)",
            );
        };
        let Some(description) = args
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            return vak_tools::ToolOutput::error("missing required argument 'description'");
        };
        let Some(instructions) = args
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            return vak_tools::ToolOutput::error("missing required argument 'instructions'");
        };
        if description.is_empty() || instructions.is_empty() {
            return vak_tools::ToolOutput::error(
                "'description' and 'instructions' must not be empty",
            );
        }

        let id = uuid::Uuid::now_v7().simple().to_string();
        let dir = proposals_dir(
            &vak_config::scope::AgentScope::new(&self.sessions_home),
            &self.cwd,
        );
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return vak_tools::ToolOutput::error(format!("create proposals dir: {e}"));
        }
        let dup_line = match duplicate_of(
            &name,
            prose(instructions),
            &accepted_skill_bodies(
                &vak_config::scope::AgentScope::new(&self.sessions_home),
                &self.cwd,
            ),
        ) {
            Some(dup) => format!("{DUPLICATE_KEY}: \"{dup}\"\n"),
            None => String::new(),
        };
        let body = format!(
            "---\nname: \"{name}\"\ndescription: \"{desc}\"\n{dup_line}---\n\n{instr}\n\n<!-- proposed-by: {sid} at {ts}; proposal id {id} -->\n",
            desc = description.replace('"', "'"),
            instr = instructions,
            sid = self.session_id,
            ts = chrono::Utc::now().to_rfc3339(),
        );
        let path = dir.join(format!("{id}.md"));
        if let Err(e) = std::fs::write(&path, body) {
            return vak_tools::ToolOutput::error(format!("write proposal: {e}"));
        }
        vak_tools::ToolOutput::ok(format!(
            "skill '{name}' queued for review as proposal {id}. It will NOT be \
             available until the user promotes it."
        ))
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: true,
            read_only: false,
            paths: vec![],
        }
    }
}

/// Kebab-case enforcement: lowercase letters/digits/dashes only, at least
/// one letter, no leading/trailing dash.
pub fn sanitize_name(raw: &str) -> Option<String> {
    let name = raw.trim().to_lowercase();
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || name.starts_with('-')
        || name.ends_with('-')
        || !name.chars().any(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    Some(name)
}

// ---- Review queue API -------------------------------------------------------

fn list_proposals_in_dir(
    dir: &Path,
    scope: &vak_config::scope::AgentScope,
    cwd: &Path,
) -> Vec<SkillProposal> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        let Some(skill) = crate::skills::parse(&path) else {
            continue;
        };
        found.push((id, path, skill));
    }
    let mut out = Vec::with_capacity(found.len());
    if !found.is_empty() {
        let accepted = accepted_skill_bodies(scope, cwd);
        for (id, path, skill) in found {
            let tag = screen_proposal(&path, &skill.name, &accepted);
            out.push(SkillProposal {
                id,
                name: skill.name,
                description: with_duplicate_note(&skill.description, tag.as_deref()),
                derived_from: std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|b| proposal_provenance(&b)),
                path,
            });
        }
    }
    out
}

/// Pending drafts, newest first. When a proposal screens as a near copy of
/// an accepted skill, the returned `description` carries a
/// `[duplicate-of: <name>]` suffix (every review surface renders the
/// description) and the flag persists as a `duplicate-of:` frontmatter line
/// so hand edits and later listings agree.
pub fn list_proposals(scope: &vak_config::scope::AgentScope, cwd: &Path) -> Vec<SkillProposal> {
    let mut proposals = list_proposals_in_dir(&proposals_dir(scope, cwd), scope, cwd);
    if proposals.is_empty() {
        let agents_dir = scope.agents_dir();
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let agent_props = list_proposals_in_dir(
                        &proposals_dir(&vak_config::scope::AgentScope::new(&p), cwd),
                        scope,
                        cwd,
                    );
                    proposals.extend(agent_props);
                }
            }
        }
    }
    proposals.sort_by(|a, b| b.id.cmp(&a.id));
    proposals
}

/// Install a proposal into user-level discovery. Refuses to silently
/// overwrite an existing skill of the same name.
pub fn promote(
    scope: &vak_config::scope::AgentScope,
    cwd: &Path,
    id: &str,
) -> Result<String, String> {
    let proposals = list_proposals(scope, cwd);
    let p = proposals
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no proposal '{id}'"))?;
    let target_dir = scope.skill(&p.name);
    let target = target_dir.join("SKILL.md");
    if target.exists() {
        return Err(format!(
            "a skill named '{}' already exists at {}; remove or rename it first",
            p.name,
            target.display()
        ));
    }
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("create skill dir: {e}"))?;
    std::fs::copy(&p.path, &target).map_err(|e| format!("install skill: {e}"))?;
    std::fs::remove_file(&p.path).map_err(|e| format!("remove pending file: {e}"))?;
    Ok(p.name.clone())
}

pub fn reject(scope: &vak_config::scope::AgentScope, cwd: &Path, id: &str) -> Result<(), String> {
    let proposals = list_proposals(scope, cwd);
    let p = proposals
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no proposal '{id}'"))?;
    std::fs::remove_file(&p.path).map_err(|e| format!("remove proposal: {e}"))
}

// ---- duplicate screening (docs/design/29-personal-os.md P5) -----------------

/// Same bar as reflection's note dedup — skill pollution is the same failure
/// mode at proposal time.
const SKILL_DEDUP_THRESHOLD: f32 = 0.55;

/// Name of the first existing skill (name, body) whose token-Jaccard
/// similarity against the candidate's title+body reaches
/// [`SKILL_DEDUP_THRESHOLD`]. Pure screening: callers tag the proposal
/// `duplicate-of`; nothing is auto-deleted.
pub fn duplicate_of(
    candidate_title: &str,
    candidate_body: &str,
    existing: &[(String, String)],
) -> Option<String> {
    let candidate = format!("{candidate_title}\n{candidate_body}");
    existing
        .iter()
        .find(|(name, body)| {
            crate::reflection::jaccard(&format!("{name}\n{body}"), &candidate)
                >= SKILL_DEDUP_THRESHOLD
        })
        .map(|(name, _)| name.clone())
}

/// Frontmatter key persisted on flagged proposals; consumers' parsers skip
/// unknown keys, so the line is inert everywhere but machine-readable.
const DUPLICATE_KEY: &str = "duplicate-of";

/// Instructions-only view of a markdown body; provenance comments are noise
/// for similarity scoring.
fn prose(body: &str) -> &str {
    match body.split_once("<!--") {
        Some((head, _)) => head,
        None => body,
    }
    .trim()
}

/// Header (between the opening and closing `---`) plus remainder of a
/// house-style markdown file.
fn split_header(text: &str) -> Option<(&str, &str)> {
    text.strip_prefix("---")
        .and_then(|rest| rest.split_once("---"))
}

fn duplicate_tag_in(frontmatter: &str) -> Option<String> {
    frontmatter.lines().find_map(|line| {
        line.trim()
            .strip_prefix(DUPLICATE_KEY)
            .and_then(|rest| rest.strip_prefix(':'))
            .map(|value| value.trim().trim_matches('"').to_string())
    })
}

/// Insert the tag as the last frontmatter line. Malformed headers are left
/// untouched — screening must never corrupt a reviewable draft.
fn persist_duplicate_tag(path: &Path, dup: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let Some((frontmatter, body)) = split_header(&text) else {
        return Ok(());
    };
    if duplicate_tag_in(frontmatter).is_some() {
        return Ok(());
    }
    let mut out = String::with_capacity(text.len() + DUPLICATE_KEY.len() + dup.len() + 5);
    out.push_str("---");
    out.push_str(frontmatter);
    if !frontmatter.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(DUPLICATE_KEY);
    out.push_str(": \"");
    out.push_str(dup);
    out.push_str("\"\n---");
    out.push_str(body);
    std::fs::write(path, out)
}

/// Existing-or-newly-persisted duplicate flag for one pending proposal.
fn screen_proposal(path: &Path, name: &str, accepted: &[(String, String)]) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let (frontmatter, body) = split_header(&text)?;
    if let Some(tag) = duplicate_tag_in(frontmatter) {
        return Some(tag);
    }
    let dup = duplicate_of(name, prose(body), accepted)?;
    persist_duplicate_tag(path, &dup).ok()?;
    Some(dup)
}

fn with_duplicate_note(description: &str, dup: Option<&str>) -> String {
    match dup {
        Some(name) => format!("{description} [duplicate-of: {name}]"),
        None => description.to_string(),
    }
}

/// (name, instructions) pairs for every discovered project/user skill — the
/// same roots skills::discover walks, read-only from this side.
fn accepted_skill_bodies(
    scope: &vak_config::scope::AgentScope,
    cwd: &Path,
) -> Vec<(String, String)> {
    let mut roots = vec![
        vak_config::scope::WorkspaceScope::new(cwd).skills(),
        scope.skills(),
    ];
    let agents_dir = scope.agents_dir();
    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                roots.push(vak_config::scope::AgentScope::new(&p).skills());
            }
        }
    }
    roots.dedup();
    let mut out: Vec<(String, String)> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path().join("SKILL.md");
            if !path.is_file() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(skill) = crate::skills::parse(&path) {
                let body = split_header(&text).map(|(_, body)| body).unwrap_or(&text);
                out.push((skill.name, prose(body).to_string()));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use vak_tools::Tool as _;

    #[test]
    fn duplicate_of_flags_identical_skill() {
        let existing = vec![(
            "rotate-release-tags".to_string(),
            "pause before rollback so the deploy script can finish".to_string(),
        )];
        assert_eq!(
            duplicate_of(
                "rotate-release-tags",
                "pause before rollback so the deploy script can finish",
                &existing,
            ),
            Some("rotate-release-tags".to_string())
        );
    }

    #[test]
    fn duplicate_of_catches_paraphrase_over_threshold() {
        let existing = vec![(
            "rotate-release-tags".to_string(),
            "pause before rollback so the deploy script can finish".to_string(),
        )];
        assert_eq!(
            duplicate_of(
                "rotate release tags",
                "pause before rollback lets the deploy script finish",
                &existing,
            ),
            Some("rotate-release-tags".to_string())
        );
    }

    #[test]
    fn duplicate_of_allows_disjoint_skills() {
        let existing = vec![
            (
                "frobulate-widget-frames".to_string(),
                "wedge alignment for panel mounts".to_string(),
            ),
            (
                "rotate-release-tags".to_string(),
                "pause before rollback so the deploy script can finish".to_string(),
            ),
        ];
        assert_eq!(
            duplicate_of(
                "quixotic-lantern-parade",
                "spinning light festival route",
                &existing,
            ),
            None
        );
        assert!(
            duplicate_of(
                "quixotic-lantern-parade",
                "spinning light festival route",
                &[],
            )
            .is_none()
        );
    }

    #[test]
    fn duplicate_of_returns_first_matching_existing() {
        let dup = (
            "rotate-release-tags".to_string(),
            "pause before rollback so the deploy script can finish".to_string(),
        );
        let existing = vec![(
            "unrelated-thing".to_string(),
            "totally different domain words here".to_string(),
        )];
        assert_eq!(
            duplicate_of("rotate-release-tags", dup.1.as_str(), &existing),
            None
        );

        let mut both = existing;
        both.push(dup);
        assert_eq!(
            duplicate_of(
                "rotate-release-tags",
                "pause before rollback so the deploy script can finish",
                &both,
            ),
            Some("rotate-release-tags".to_string())
        );
    }

    // ---- submission/review integration ----

    fn seed_accepted_skill(home: &Path, name: &str, desc: &str, body: &str) {
        let dir = vak_config::scope::AgentScope::new(home).skill(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: \"{name}\"\ndescription: \"{desc}\"\n---\n\n{body}\n"),
        )
        .unwrap();
    }

    fn pending_paths(home: &Path, cwd: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(proposals_dir(
            &vak_config::scope::AgentScope::new(home),
            cwd,
        ))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
        v.sort();
        v
    }

    fn temp_home_cwd() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        (dir, home, cwd)
    }

    #[tokio::test]
    async fn propose_submission_tags_duplicate_and_promotion_still_works() {
        let (_dir, home, cwd) = temp_home_cwd();
        seed_accepted_skill(
            &home,
            "rotate-release-tags",
            "pause deploys during rollback windows",
            "pause before rollback so the deploy script can finish",
        );

        let tool = ProposeSkillTool {
            sessions_home: home.clone(),
            cwd: cwd.clone(),
            session_id: "sess-a".into(),
        };
        let ctx = vak_tools::ToolContext::new(cwd.clone());
        let args = serde_json::json!({
            "name": "rotate-release-tags-v2",
            "description": "wait out the deploy window",
            "instructions": "always pause before rollback lets the deploy script finish"
        });
        assert!(!tool.execute(&args, &ctx).await.is_error);
        // Re-proposal queues a fresh draft; neither may stack tag lines.
        assert!(!tool.execute(&args, &ctx).await.is_error);

        for path in pending_paths(&home, &cwd) {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("duplicate-of: \"rotate-release-tags\""));
            assert_eq!(
                text.matches("duplicate-of:").count(),
                1,
                "exactly one tag line in {}",
                path.display()
            );
            // Stored description stays pristine; the flag lives on its own line.
            assert!(text.contains("description: \"wait out the deploy window\""));
            assert!(!text.contains("[duplicate-of:"));
        }

        let listed = list_proposals(&vak_config::scope::AgentScope::new(&home), &cwd);
        assert_eq!(listed.len(), 2);
        for p in &listed {
            assert!(
                p.description
                    .ends_with("[duplicate-of: rotate-release-tags]"),
                "{}",
                p.description
            );
        }

        // A flagged proposal is still promotable by explicit human decision.
        let id = listed[0].id.clone();
        assert_eq!(
            promote(&vak_config::scope::AgentScope::new(&home), &cwd, &id).unwrap(),
            "rotate-release-tags-v2"
        );
        let installed = home.join("skills/rotate-release-tags-v2/SKILL.md");
        assert!(installed.exists());
        let parsed = crate::skills::parse(&installed).unwrap();
        assert_eq!(parsed.description, "wait out the deploy window");
        assert!(!parsed.description.contains("duplicate-of"));
    }

    #[tokio::test]
    async fn propose_submission_leaves_disjoint_skills_untagged() {
        let (_dir, home, cwd) = temp_home_cwd();
        seed_accepted_skill(
            &home,
            "frobulate-widget-frames",
            "wedge alignment for panel mounts",
            "wedge alignment for panel mounts",
        );

        let tool = ProposeSkillTool {
            sessions_home: home.clone(),
            cwd: cwd.clone(),
            session_id: "sess-b".into(),
        };
        let ctx = vak_tools::ToolContext::new(cwd.clone());
        let out = tool
            .execute(
                &serde_json::json!({
                    "name": "quixotic-lantern-parade",
                    "description": "festival logistics",
                    "instructions": "spinning light festival route planning"
                }),
                &ctx,
            )
            .await;
        assert!(!out.is_error);

        for path in pending_paths(&home, &cwd) {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(!text.contains("duplicate-of"), "{text}");
        }
        let listed = list_proposals(&vak_config::scope::AgentScope::new(&home), &cwd);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].description, "festival logistics");
    }

    #[test]
    fn reflection_authored_draft_is_screened_on_review_without_stacking() {
        let (_dir, home, cwd) = temp_home_cwd();
        seed_accepted_skill(
            &home,
            "rotate-release-tags",
            "pause deploys during rollback windows",
            "pause before rollback so the deploy script can finish",
        );

        // Exact write format of the reflection queue entry point.
        let dir = proposals_dir(&vak_config::scope::AgentScope::new(&home), &cwd);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("aaa111.md");
        std::fs::write(
            &file,
            "---\nname: \"rotate-release-tags-v3\"\ndescription: \"hold deploys at the rollback gate\"\n---\n\npause before rollback so the deploy script can finish cleanly\n\n<!-- proposed-by: sess-r at 2026-01-01T00:00:00+00:00; proposal id aaa111; source: reflection -->\n",
        )
        .unwrap();

        let first = list_proposals(&vak_config::scope::AgentScope::new(&home), &cwd);
        assert_eq!(first.len(), 1);
        assert!(
            first[0]
                .description
                .contains("[duplicate-of: rotate-release-tags]"),
            "{}",
            first[0].description
        );
        assert_eq!(
            std::fs::read_to_string(&file)
                .unwrap()
                .matches("duplicate-of:")
                .count(),
            1
        );

        // Repeat listings never stack a second tag line.
        let second = list_proposals(&vak_config::scope::AgentScope::new(&home), &cwd);
        assert_eq!(second[0].description, first[0].description);
        assert_eq!(
            std::fs::read_to_string(&file)
                .unwrap()
                .matches("duplicate-of:")
                .count(),
            1
        );

        // Rejection is unchanged for flagged drafts.
        assert!(reject(&vak_config::scope::AgentScope::new(&home), &cwd, "aaa111").is_ok());
        assert!(list_proposals(&vak_config::scope::AgentScope::new(&home), &cwd).is_empty());
    }

    #[test]
    fn tagged_header_is_inert_to_consumer_parsers() {
        let (_dir, home, cwd) = temp_home_cwd();
        let dir = proposals_dir(&vak_config::scope::AgentScope::new(&home), &cwd);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bbb222.md");
        std::fs::write(
            &file,
            "---\nname: \"hand-tagged\"\ndescription: \"original text\"\nduplicate-of: \"rotate-release-tags\"\n---\n\nbody here\n\n<!-- proposed-by: sess-h at ts; proposal id bbb222 -->\n",
        )
        .unwrap();

        // Replicates skills::parse as used by discovery and every listing:
        // unknown frontmatter keys are skipped, name/description untouched.
        let parsed = crate::skills::parse(&file).expect("tagged header still parses");
        assert_eq!(parsed.name, "hand-tagged");
        assert_eq!(parsed.description, "original text");

        // Replicates the server payload shape and TUI/CLI row rendering,
        // which all show the flag via description without their own changes.
        let listed = list_proposals(&vak_config::scope::AgentScope::new(&home), &cwd);
        let p = &listed[0];
        let payload = serde_json::json!({"id": p.id, "name": p.name, "description": p.description});
        assert_eq!(payload["name"], "hand-tagged");
        assert!(
            payload["description"]
                .as_str()
                .unwrap()
                .contains("[duplicate-of: rotate-release-tags]"),
            "{}",
            payload["description"]
        );
    }
}
