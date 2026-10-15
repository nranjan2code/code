//! A runnable skill's verification (docs/design/93-self-evolving-
//! capabilities.md §4, stage E2): the runtime runs the program's own tests
//! through the path every call takes (`vak_tools::capability`, sandboxed
//! by its declaration), in a run of its own, and keeps the result per
//! program digest. A test's verdict is what the runtime observed, never
//! what a model said of it (invariant 33).

use std::path::Path;

use serde::{Deserialize, Serialize};
use vak_tools::Tool as _;
use vak_tools::capability::{CapabilityTool, Program, Writes};

use crate::Core;

/// One test case as the runtime ran it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    pub name: String,
    pub passed: bool,
    /// Where the answer differed, or how the call failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A program's tests as one run found them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verification {
    pub skill: String,
    /// The program verified: a digest over every file of the skill.
    pub digest: String,
    pub run: vak_session::ids::RunId,
    pub at: chrono::DateTime<chrono::Utc>,
    pub cases: Vec<CaseResult>,
}

impl Verification {
    pub fn passed(&self) -> bool {
        self.cases.iter().all(|case| case.passed)
    }
}

impl Core {
    /// Runs the tests of the skill `name` in `dir` and records what they
    /// found under the program's digest.
    pub async fn verify_skill_program(
        &self,
        name: &str,
        dir: &Path,
    ) -> Result<Verification, String> {
        let program =
            Program::load(dir)?.ok_or_else(|| format!("'{name}' has no program to verify"))?;
        let trace = self.mint_trace(None);
        let mut run = self
            .runs()
            .begin(&trace, None, None)
            .map_err(|error| format!("the verification run did not open: {error}"))?;
        let tool = CapabilityTool::new(
            name.to_string(),
            String::new(),
            program.clone(),
            self.tool_worker_exe(),
        );
        let agent = self
            .agent_identity()
            .map_or_else(|| "vak".to_string(), |agent| agent.id.clone());
        let ctx = vak_tools::ToolContext::new(self.cwd().to_path_buf())
            .with_agent_id(agent)
            .with_trace(trace.child());
        let limit_ms = self.time_limits().command_secs.saturating_mul(1000);
        let cases = tracing::Instrument::instrument(
            async {
                let mut cases = Vec::new();
                for (case_name, case) in &program.tests {
                    let mut args = case.arguments.clone();
                    if let Some(object) = args.as_object_mut() {
                        object.insert("timeout_ms".into(), limit_ms.into());
                    }
                    let output = tool.execute(&args, &ctx).await;
                    let judged = case.judge(&output, program.writes == Writes::Drafts);
                    cases.push(CaseResult {
                        name: case_name.clone(),
                        passed: judged.is_ok(),
                        detail: judged.err(),
                    });
                }
                cases
            },
            vak_session::runs::span(&trace),
        )
        .await;
        let verification = Verification {
            skill: name.to_string(),
            digest: program.digest.clone(),
            run: run.id(),
            at: chrono::Utc::now(),
            cases,
        };
        let failed = verification
            .cases
            .iter()
            .filter(|case| !case.passed)
            .count();
        run.settle_with(
            if failed == 0 {
                vak_session::runs::RunOutcome::Completed
            } else {
                vak_session::runs::RunOutcome::Failed {
                    reason: format!("{failed} of {} tests failed", verification.cases.len()),
                }
            },
            None,
        );
        let text = serde_json::to_string(&verification)
            .map_err(|error| format!("the verification did not encode: {error}"))?;
        // A program verified again (proposed again unchanged) keeps each
        // run as a new version of its digest's record; the latest is read.
        vak_session::documents::update(&self.verification_path(&program.digest), |_| {
            Ok(Some((text.clone(), ())))
        })?;
        Ok(verification)
    }

    /// The latest verification recorded for the program with `digest`.
    pub fn skill_program_verification(&self, digest: &str) -> Option<Verification> {
        vak_session::documents::read(&self.verification_path(digest))
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str(&text).ok())
    }

    fn verification_path(&self, digest: &str) -> std::path::PathBuf {
        self.scope().capability_checks(self.cwd()).join(digest)
    }
}

/// What proposing a skill with a program came to.
#[derive(Debug, Clone)]
pub struct Proposed {
    pub verification: Verification,
    /// The Library artifact and version the skill was recorded as.
    pub artifact: String,
    pub version: String,
    /// Where it stands: live for the Agent, waiting for the owner, or
    /// refused because its tests failed.
    pub standing: Standing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// Installed among the Agent's own skills: offered from its next turn.
    Live,
    /// In the review queue, as proposal `id`, for the reason given.
    Review { id: String, why: String },
    /// Not admitted: a test failed. The version is kept in the Library.
    Failed,
}

/// A skill proposal with a program: the folder the Agent prepared, and the
/// words the skill is offered under.
pub struct ProgramProposal<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub instructions: &'a str,
    /// The kinds of work it serves (`serves:` in its SKILL.md).
    pub serves: &'a [String],
    pub folder: &'a Path,
    pub session: &'a str,
    pub call: &'a str,
    /// What the proposing turn held (invariant 43).
    pub held: vak_control::Provenance,
}

/// Why a verified program waits for the owner instead of going live, from
/// what it declares, what the proposing turn held and the owner's
/// `abilities_go_live` (doc 93 §5); `None` when it may go live.
pub fn review_reason(
    program: &Program,
    held: vak_control::Provenance,
    go_live: bool,
) -> Option<String> {
    if !program.hosts.is_empty() {
        return Some(format!("it connects to {}", program.hosts.join(", ")));
    }
    if let Some(requirements) = &program.requirements
        && !requirements.installed()
    {
        return Some(format!(
            "it installs packages from {}",
            requirements.hosts.join(", ")
        ));
    }
    if held.untrusted {
        return Some("it was written in a turn that read someone else's words".into());
    }
    if !go_live {
        return Some("the owner reviews every new ability".into());
    }
    None
}

/// What a proposal waiting for review records about its program, so every
/// surface that shows it says why it waits and what it would reach
/// without reading the bundle again (doc 93 §9.2, §9.5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProgramReview {
    pub why: String,
    pub reaches: Vec<String>,
    pub artifact: String,
    pub version: String,
}

const REVIEW_TRAILER: &str = "<!-- program-review: ";

impl ProgramReview {
    fn trailer(&self) -> String {
        format!(
            "{REVIEW_TRAILER}{} -->",
            serde_json::to_string(self).unwrap_or_default()
        )
    }

    /// The record a proposal's body carries, if it has a program.
    pub fn read(body: &str) -> Option<Self> {
        let line = body
            .lines()
            .rev()
            .find(|line| line.starts_with(REVIEW_TRAILER))?;
        serde_json::from_str(line.strip_prefix(REVIEW_TRAILER)?.strip_suffix(" -->")?).ok()
    }
}

/// The `serves:` line of a skill's frontmatter; none when it declares none.
pub fn serves_line(serves: &[String]) -> String {
    if serves.is_empty() {
        String::new()
    } else {
        format!("serves: [{}]\n", serves.join(", "))
    }
}

/// The inbox key of the entry that says proposal `id` is waiting.
pub fn waiting_key(id: &str) -> String {
    format!("proposal-waiting|{id}")
}

/// Marks read the entry that said proposal `id` was waiting, once the owner
/// approved or declined it.
fn settle_waiting(scope: &vak_config::scope::AgentScope, id: &str) {
    let key = waiting_key(id);
    for entry in crate::inbox::unread(scope) {
        if entry.dedupe_key.as_deref() == Some(key.as_str()) {
            let _ = crate::inbox::ack(scope, &entry.id);
        }
    }
}

impl Core {
    /// The owner approved or declined proposal `id`: the inbox no longer
    /// says it is waiting.
    pub fn proposal_settled(&self, id: &str) {
        settle_waiting(&self.shared_scope().as_agent(), id);
    }

    /// Checks, verifies, records and admits a skill with a program
    /// (doc 93 §4): the folder is copied as one bundle with the skill's
    /// `SKILL.md` written from the proposal's own words, its tests run in a
    /// run of their own, the bundle becomes a Library version (its parent
    /// the skill's latest version, so improving a skill is proposing it
    /// again under its name), and the tier decides whether it goes live
    /// for the Agent or waits for the owner.
    pub async fn propose_skill_program(
        &self,
        proposal: ProgramProposal<'_>,
    ) -> Result<Proposed, String> {
        let name = proposal.name;
        let staging_root = vak_config::paths::runtime_dir()
            .join("skill-proposals")
            .join(uuid::Uuid::now_v7().simple().to_string());
        let staging = staging_root.join(name);
        let result = self.propose_from(&proposal, &staging).await;
        let _ = std::fs::remove_dir_all(&staging_root);
        result
    }

    async fn propose_from(
        &self,
        proposal: &ProgramProposal<'_>,
        staging: &Path,
    ) -> Result<Proposed, String> {
        let name = proposal.name;
        if !proposal
            .folder
            .join(vak_tools::capability::DECLARATION)
            .is_file()
        {
            return Err(format!(
                "{} has no {}: `source` is for a skill with a program; for instructions alone leave it out",
                proposal.folder.display(),
                vak_tools::capability::DECLARATION
            ));
        }
        vak_tools::capability::unbundle(&vak_tools::capability::bundle(proposal.folder)?, staging)?;
        let skill_md = format!(
            "---\nname: \"{name}\"\ndescription: \"{}\"\n{}---\n\n{}\n",
            proposal.description.replace('"', "'"),
            serves_line(proposal.serves),
            proposal.instructions
        );
        std::fs::write(staging.join("SKILL.md"), &skill_md)
            .map_err(|error| format!("cannot write the skill: {error}"))?;
        let program = Program::load(staging)?
            .ok_or_else(|| format!("{} did not load", vak_tools::capability::DECLARATION))?;
        crate::presentation_store::skill_packs(staging)?;
        let verification = self.verify_skill_program(name, staging).await?;
        let bytes = vak_tools::capability::bundle(staging)?;

        let trace = self.mint_trace(None);
        let agent = self
            .agent_identity()
            .map_or_else(|| "vak".to_string(), |agent| agent.id.clone());
        let space = vak_session::trace::local::space(self.cwd()).to_string();
        let path = format!("skills/{name}");
        let (artifact, version) = self
            .artifacts()
            .transaction(|tx| {
                let id = tx.declare(
                    &space,
                    &agent,
                    &path,
                    crate::artifacts::ArtifactKind::Skill,
                    Some(name.to_string()),
                    Some(proposal.description.to_string()),
                    Some(&trace),
                    None,
                )?;
                let version = tx.version(
                    id,
                    crate::artifacts::NewVersion {
                        parent: None,
                        bytes: &bytes,
                        source: crate::artifacts::VersionSource::Call {
                            session: proposal.session.to_string(),
                            call: proposal.call.to_string(),
                            in_workspace: false,
                        },
                    },
                    Some(&trace),
                    None,
                )?;
                tx.record(
                    id,
                    crate::artifacts::ArtifactStep::Verified {
                        version,
                        run: verification.run.to_string(),
                        passed: verification.passed(),
                    },
                    Some(&trace),
                    None,
                )?;
                Ok((id, version))
            })
            .map_err(|error| format!("the skill was not recorded in the Library: {error}"))?;

        let standing = if !verification.passed() {
            Standing::Failed
        } else if let Some(why) = review_reason(&program, proposal.held, self.abilities_go_live()) {
            let id = uuid::Uuid::now_v7().simple().to_string();
            let dir = self.scope().skill_proposals(self.cwd());
            let review = ProgramReview {
                why: why.clone(),
                reaches: program.reaches(),
                artifact: artifact.to_string(),
                version: version.to_string(),
            };
            let body = format!(
                "{skill_md}\n<!-- proposed-by: {} at {}; proposal id {id}; library: {artifact} {version} -->\n{}\n",
                proposal.session,
                chrono::Utc::now().to_rfc3339(),
                review.trailer()
            );
            vak_session::documents::create(&dir.join(format!("{id}.md")), &body)?;
            let _ = crate::inbox::record_with_result_and_key(
                &self.shared_scope().as_agent(),
                crate::inbox::Kind::ProposalOpened,
                &format!("{name} is waiting for your review"),
                &format!(
                    "{}\nIt waits because {why}.\n{}",
                    proposal.description.trim(),
                    review
                        .reaches
                        .iter()
                        .map(|line| format!("- {line}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
                Some(proposal.session),
                None,
                None,
                Some(&waiting_key(&id)),
                Some(&trace),
            );
            Standing::Review { id, why }
        } else {
            install(&bytes, &self.scope().skills().join(name))?;
            Standing::Live
        };
        Ok(Proposed {
            verification,
            artifact: artifact.to_string(),
            version: version.to_string(),
            standing,
        })
    }
}

/// Puts a skill bundle at `target`, replacing what was there in one rename,
/// so a turn never reads half a skill. What it replaced is the Library's
/// earlier version.
pub fn install(bytes: &[u8], target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "a skill folder has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot install the skill: {error}"))?;
    let fresh = parent.join(format!(".{}.new", uuid::Uuid::now_v7().simple()));
    vak_tools::capability::unbundle(bytes, &fresh)?;
    let old = parent.join(format!(".{}.old", uuid::Uuid::now_v7().simple()));
    let had = target.exists();
    if had {
        std::fs::rename(target, &old)
            .map_err(|error| format!("cannot install the skill: {error}"))?;
    }
    if let Err(error) = std::fs::rename(&fresh, target) {
        if had {
            let _ = std::fs::rename(&old, target);
        }
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(format!("cannot install the skill: {error}"));
    }
    if had {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(())
}
