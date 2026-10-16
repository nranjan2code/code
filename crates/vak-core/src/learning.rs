//! Learning loop (docs/design/26-learning.md): `remember` appends durable
//! notes, `forget_memory` removes them, and `propose_skill` queues skill drafts.
//! Proposals never enter discovery by themselves — promotion is an explicit
//! human action over HTTP or CLI.

use std::path::{Path, PathBuf};
use vak_tools::Fault;

use serde_json::Value;

use crate::memory;

const KINDS: [&str; 5] = ["fact", "decision", "preference", "reference", "invariant"];

// ---- remember ---------------------------------------------------------------

pub struct RememberTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
}

/// Remove one curated memory note after `recall` identifies its
/// stable id. Session ledgers remain append-only; this edits only MEMORY.md.
pub struct ForgetMemoryTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
}

impl ForgetMemoryTool {
    /// What this tool declares to the permission engine, known before one
    /// is built.
    pub const PERMISSION: vak_permission::Declared =
        vak_permission::Declared::reach(vak_permission::Reach::Learning);
}

#[async_trait::async_trait]
impl vak_tools::Tool for ForgetMemoryTool {
    fn permission(&self) -> vak_permission::Declared {
        Self::PERMISSION
    }

    fn name(&self) -> &str {
        "forget_memory"
    }

    fn serves(&self) -> Vec<&str> {
        vec!["memory"]
    }

    fn description(&self) -> &str {
        "Forget one saved memory note when the user asks to delete or forget it. First find the note with recall, by its topic; pass the memory/<id> or profile/<id> id its hit gives here. If the result is ambiguous, ask which note they mean. This removes the note from durable memory and future memory search; it does not erase the conversation that originally created it."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "note_id": {"type": "string", "description": "The exact memory/<id> or profile/<id> id a recall hit gave."}
            },
            "required": ["note_id"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(source_id) = args.get("note_id").and_then(Value::as_str) else {
            return vak_tools::ToolOutput::error(
                Fault::Shape,
                "missing required argument 'note_id'",
            );
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
                Fault::Shape,
                "note_id must be the exact memory/<id> or profile/<id> id a recall hit gave",
            );
        };
        match result {
            Ok(_) => vak_tools::ToolOutput::ok(format!(
                "Forgot the {scope} memory note. It will no longer appear in future memory searches; its source conversation remains in history."
            )),
            Err(error) => vak_tools::ToolOutput::error(
                Fault::Tool,
                format!("could not forget memory note: {error}"),
            ),
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

impl ProposeSkillTool {
    async fn propose_program(
        &self,
        name: &str,
        description: &str,
        instructions: &str,
        source: &str,
        serves: &[String],
        ctx: &vak_tools::ToolContext,
    ) -> vak_tools::ToolOutput {
        use crate::skill_programs::{ProgramProposal, Standing};
        let Some(core) = &self.core else {
            return vak_tools::ToolOutput::error(
                Fault::Refused,
                "a skill with a program cannot be proposed here; leave out `source`",
            );
        };
        let folder = match vak_tools::drafts::existing(ctx, source) {
            Ok(folder) if folder.is_dir() => folder,
            Ok(_) => {
                return vak_tools::ToolOutput::error(
                    Fault::Target,
                    format!("{source} is not a folder"),
                );
            }
            Err(error) => return vak_tools::ToolOutput::error(Fault::Target, error),
        };
        let call = ctx
            .sandbox_sink
            .as_ref()
            .map(|sink| sink.execution_id().to_string())
            .unwrap_or_default();
        let proposed = match core
            .propose_skill_program(ProgramProposal {
                name,
                description,
                instructions,
                serves,
                folder: &folder,
                session: &self.session_id,
                call: &call,
                held: ctx.provenance,
                unattended: crate::skill_programs::unattended(ctx.trace.as_ref()),
            })
            .await
        {
            Ok(proposed) => proposed,
            Err(error) => return vak_tools::ToolOutput::error(Fault::Shape, error),
        };
        let cases = &proposed.verification.cases;
        let passed = cases.iter().filter(|case| case.passed).count();
        let failures: Vec<String> = cases
            .iter()
            .filter_map(|case| {
                case.detail
                    .as_ref()
                    .map(|detail| format!("- {}: {detail}", case.name))
            })
            .collect();
        match proposed.standing {
            Standing::Live => vak_tools::ToolOutput::ok(format!(
                "skill '{name}' passed {passed} of {} tests and is yours from your next turn \
                 (Library version {}).",
                cases.len(),
                proposed.version
            )),
            Standing::Review { id, why } => vak_tools::ToolOutput::ok(format!(
                "skill '{name}' passed {passed} of {} tests and waits for the person's review \
                 as proposal {id}, because {why}. It is NOT available until they approve it.",
                cases.len()
            )),
            Standing::Failed => vak_tools::ToolOutput::ok(format!(
                "skill '{name}' was not admitted: {} of {} tests failed:\n{}\nFix the program or \
                 its tests and propose it again.",
                cases.len() - passed,
                cases.len(),
                failures.join("\n")
            )),
        }
    }
}

impl RememberTool {
    /// What this tool declares to the permission engine, known before one
    /// is built.
    pub const PERMISSION: vak_permission::Declared =
        vak_permission::Declared::reach(vak_permission::Reach::Learning);
}

#[async_trait::async_trait]
impl vak_tools::Tool for RememberTool {
    fn permission(&self) -> vak_permission::Declared {
        Self::PERMISSION
    }

    fn name(&self) -> &str {
        "remember"
    }

    fn serves(&self) -> Vec<&str> {
        vec!["memory"]
    }

    fn always_loaded(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Keep a note for later conversations: whatever the person asks you to \
         remember (their name, a preference, a fact about them or their work), \
         and decisions worth knowing next time. Later conversations find it \
         with recall; the person can see, edit or forget it. When asked to \
         forget a note, find its exact id with recall, then \
         call forget_memory; do not replace it with a note saying not to \
         remember it."
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
            return vak_tools::ToolOutput::error(Fault::Shape, "missing required argument 'note'");
        };
        let kind = args.get("kind").and_then(Value::as_str).unwrap_or("fact");
        if !KINDS.contains(&kind) {
            return vak_tools::ToolOutput::error(
                Fault::Shape,
                format!("unknown kind '{kind}'; expected one of {KINDS:?}"),
            );
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
                "remembered ({kind}{tag_suffix}) as memory/{}. recall finds it in later conversations; forget_memory takes that id if the person asks to forget it.",
                note.id,
                tag_suffix = if tag.is_empty() {
                    String::new()
                } else {
                    format!(", tag '{tag}'")
                }
            )),
            Err(e) => {
                vak_tools::ToolOutput::error(Fault::Tool, format!("could not persist note: {e}"))
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

// ---- propose_skill ----------------------------------------------------------

pub struct SkillProposal {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    /// The conversation (and turn, when recorded) the draft came from, read
    /// from its `proposed-by` trailer.
    pub derived_from: Option<vak_session::trace::DerivedFrom>,
    /// For a skill with a program: why it waits and what it would reach.
    pub program: Option<crate::skill_programs::ProgramReview>,
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
    /// What a proposal with a program is checked, verified, recorded and
    /// admitted through (`Core::propose_skill_program`); `None` where only
    /// instructions can be proposed.
    pub core: Option<crate::Core>,
}

impl ProposeSkillTool {
    /// What this tool declares to the permission engine, known before one
    /// is built.
    pub const PERMISSION: vak_permission::Declared =
        vak_permission::Declared::reach(vak_permission::Reach::Learning);
}

#[async_trait::async_trait]
impl vak_tools::Tool for ProposeSkillTool {
    fn permission(&self) -> vak_permission::Declared {
        Self::PERMISSION
    }

    fn name(&self) -> &str {
        "propose_skill"
    }

    fn serves(&self) -> Vec<&str> {
        vec!["memory"]
    }

    fn description(&self) -> &str {
        "Save a reusable skill from something learned this session: a \
         procedure that worked, or (with `source`) a program you wrote \
         and tested. Instructions alone go to the person's review queue. \
         With `source`, the folder's program and tests are checked and \
         run; one that passes, reaches no network and was not written \
         after reading someone else's words is yours from the next turn, \
         anything else waits for the person. Proposing a skill again under \
         its name improves it as a new version. Do not propose one-off steps."
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
                                 "description": "Markdown procedure the future agent should follow"},
                "source": {"type": "string",
                           "description": format!("Optional: a folder you wrote for a skill with a program; its SKILL.md is written from name, description and instructions. {}", vak_tools::capability::FORMAT)},
                "serves": {"type": "array", "items": {"type": "string"},
                           "description": format!(
                               "Optional: the kinds of work it serves, so it is listed by name and loaded only when called, instead of every turn: any of {}, or a word of your own",
                               crate::capability::Domain::known().iter().map(|domain| domain.as_str().to_string()).collect::<Vec<_>>().join(", ")
                           )}
            },
            "required": ["name", "description", "instructions"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(name) = sanitize_name(args.get("name").and_then(Value::as_str).unwrap_or(""))
        else {
            return vak_tools::ToolOutput::error(
                Fault::Shape,
                "'name' must be kebab-case (lowercase letters, digits, dashes)",
            );
        };
        let Some(description) = args
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            return vak_tools::ToolOutput::error(
                Fault::Shape,
                "missing required argument 'description'",
            );
        };
        let Some(instructions) = args
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            return vak_tools::ToolOutput::error(
                Fault::Shape,
                "missing required argument 'instructions'",
            );
        };
        if description.is_empty() || instructions.is_empty() {
            return vak_tools::ToolOutput::error(
                Fault::Shape,
                "'description' and 'instructions' must not be empty",
            );
        }

        let serves: Vec<String> = args
            .get("serves")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|domain| {
                        crate::capability::Domain::parse(domain)
                            .as_str()
                            .to_string()
                    })
                    .filter(|domain| {
                        !domain.is_empty()
                            && domain
                                .chars()
                                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(source) = args
            .get("source")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|source| !source.is_empty())
        {
            return self
                .propose_program(&name, description, instructions, source, &serves, ctx)
                .await;
        }
        let id = uuid::Uuid::now_v7().simple().to_string();
        let _ = vak_config::spaces::bind(&self.cwd);
        let dir = proposals_dir(
            &vak_config::scope::AgentScope::new(&self.sessions_home),
            &self.cwd,
        );
        let body = format!(
            "---\nname: \"{name}\"\ndescription: \"{desc}\"\n{serves_line}---\n\n{instr}\n\n<!-- proposed-by: {sid} at {ts}; proposal id {id} -->\n",
            serves_line = crate::skill_programs::serves_line(&serves),
            desc = description.replace('"', "'"),
            instr = instructions,
            sid = self.session_id,
            ts = chrono::Utc::now().to_rfc3339(),
        );
        if let Err(e) = vak_session::documents::create(&dir.join(format!("{id}.md")), &body) {
            return vak_tools::ToolOutput::error(Fault::Tool, format!("write proposal: {e}"));
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

fn list_proposals_in_dir(dir: &Path) -> Vec<SkillProposal> {
    let mut found = Vec::new();
    for path in vak_session::documents::under(dir) {
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        let Some(text) = vak_session::documents::read(&path).ok().flatten() else {
            continue;
        };
        let Some(skill) = crate::skills::parse_text(&text, &path) else {
            continue;
        };
        found.push((id, path, skill));
    }
    let mut out = Vec::with_capacity(found.len());
    {
        for (id, path, skill) in found {
            let body = vak_session::documents::read(&path).ok().flatten();
            out.push(SkillProposal {
                id,
                name: skill.name,
                description: skill.description,
                derived_from: body.as_deref().and_then(proposal_provenance),
                program: body
                    .as_deref()
                    .and_then(crate::skill_programs::ProgramReview::read),
                path,
            });
        }
    }
    out
}

/// Pending drafts from every folder the Agent worked in, newest first:
/// a proposal is filed under the folder its turn ran in, and the owner
/// reviews them all in one place.
pub fn list_proposals(scope: &vak_config::scope::AgentScope) -> Vec<SkillProposal> {
    let mut proposals = list_proposals_in_dir(&scope.skill_proposals_root());
    if proposals.is_empty() {
        let agents_dir = scope.agents_dir();
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let agent_props = list_proposals_in_dir(
                        &vak_config::scope::AgentScope::new(&p).skill_proposals_root(),
                    );
                    proposals.extend(agent_props);
                }
            }
        }
    }
    proposals.sort_by(|a, b| b.id.cmp(&a.id));
    proposals
}

/// The Shared layer's capability root, `~/vak-home/.vak`: where every
/// workspace discovers Shared skills, and so where a promoted one goes.
/// The Shared layer's capability root: the default workspace's project
/// directory. Every Core reads it as its shared root, and every surface that
/// installs or checks a user-scope skill or plugin resolves it here.
pub fn shared_capability_root() -> PathBuf {
    vak_config::scope::WorkspaceScope::new(vak_config::paths::default_workspace()).project_dir()
}

/// Install a proposal into `skills`, the directory discovery reads
/// (`Core::shared_skills_dir`). Refuses to silently overwrite an existing
/// skill of the same name.
pub fn promote(
    scope: &vak_config::scope::AgentScope,
    id: &str,
    skills: &Path,
    library: Option<&crate::artifacts::Artifacts>,
) -> Result<String, String> {
    let proposals = list_proposals(scope);
    let p = proposals
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no proposal '{id}'"))?;
    let target_dir = skills.join(&p.name);
    let target = target_dir.join("SKILL.md");
    let text =
        vak_session::documents::read(&p.path)?.ok_or_else(|| format!("no proposal '{id}'"))?;
    match library_version(&text) {
        Some((artifact, version)) => {
            let library = library.ok_or_else(|| {
                format!("proposal '{id}' is a skill with a program; promote it where the Library is open")
            })?;
            let found = library
                .get(&artifact)
                .ok_or_else(|| format!("proposal '{id}' names a Library skill that is gone"))?;
            let version = found
                .versions
                .iter()
                .find(|known| known.id.to_string() == version)
                .map(|known| known.id)
                .ok_or_else(|| format!("proposal '{id}' names a version that is gone"))?;
            let bytes = library
                .bytes(&found, &version)
                .map_err(|error| format!("proposal '{id}': {error}"))?;
            crate::skill_programs::install(&bytes, &target_dir)?;
        }
        // A skill of instructions alone never replaces one; a program is
        // the next version of its Library skill and replaces the copy there.
        None if target.exists() => {
            return Err(format!(
                "a skill named '{}' already exists at {}; remove or rename it first",
                p.name,
                target.display()
            ));
        }
        None => {
            std::fs::create_dir_all(&target_dir).map_err(|e| format!("create skill dir: {e}"))?;
            std::fs::write(&target, text).map_err(|e| format!("install skill: {e}"))?;
        }
    }
    vak_session::documents::forget(&p.path)?;
    Ok(p.name.clone())
}

/// The Library artifact and version a program proposal names, from its
/// `library:` trailer.
fn library_version(body: &str) -> Option<(String, String)> {
    let line = body
        .lines()
        .rev()
        .find(|line| line.contains("proposed-by:"))?;
    let rest = line.split("library:").nth(1)?;
    let mut parts = rest.trim().trim_end_matches("-->").split_whitespace();
    Some((parts.next()?.to_string(), parts.next()?.to_string()))
}

pub fn reject(scope: &vak_config::scope::AgentScope, id: &str) -> Result<(), String> {
    let proposals = list_proposals(scope);
    let p = proposals
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no proposal '{id}'"))?;
    vak_session::documents::forget(&p.path).map(|_| ())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use vak_tools::Tool as _;

    fn temp_home_cwd() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        // The workspace is opened (bound to its space), as a Core would.
        vak_config::spaces::bind(&cwd).unwrap();
        (dir, home, cwd)
    }

    #[tokio::test]
    async fn a_proposal_is_queued_as_written_and_promoted_by_the_owner() {
        vak_config::paths::isolate_home_for_tests();
        let (_dir, home, cwd) = temp_home_cwd();
        let tool = ProposeSkillTool {
            sessions_home: home.clone(),
            cwd: cwd.clone(),
            session_id: "sess-a".into(),
            core: None,
        };
        let ctx = vak_tools::ToolContext::new(cwd.clone());
        let args = serde_json::json!({
            "name": "rotate-release-tags",
            "description": "wait out the deploy window",
            "instructions": "pause before rollback so the deploy script can finish"
        });
        assert!(!tool.execute(&args, &ctx).await.is_error());

        let scope = vak_config::scope::AgentScope::new(&home);
        let listed = list_proposals(&scope);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].description, "wait out the deploy window");
        assert_eq!(
            promote(&scope, &listed[0].id, &home.join("skills"), None).unwrap(),
            "rotate-release-tags"
        );
        let installed = home.join("skills/rotate-release-tags/SKILL.md");
        let parsed = crate::skills::parse(&installed).unwrap();
        assert_eq!(parsed.description, "wait out the deploy window");
    }
}
