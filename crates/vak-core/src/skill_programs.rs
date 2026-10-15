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
    /// The `propose_skill` call that made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
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

/// One installed version of an Agent's ability, as its calls found it
/// (doc 93 R2).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VersionRecord {
    /// The program's digest: which version a call ran.
    pub digest: String,
    /// Its Library version.
    pub version: String,
    /// The digest of the version it replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The `propose_skill` call that installed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_by: Option<String>,
    pub passed: bool,
    #[serde(default)]
    pub succeeded_elsewhere: bool,
    /// The first call that failed by the program's own fault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
    /// Offered to every Agent already.
    #[serde(default)]
    pub offered: bool,
}

impl VersionRecord {
    pub(crate) fn facts(&self) -> vak_judge::ability::VersionFacts {
        vak_judge::ability::VersionFacts {
            passed: self.passed,
            succeeded_elsewhere: self.succeeded_elsewhere,
            failed: self.failed.is_some(),
        }
    }
}

/// An Agent's learned ability: its installed versions, newest last, and
/// whether it was withdrawn.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AbilityRecord {
    pub versions: Vec<VersionRecord>,
    #[serde(default)]
    pub withdrawn: bool,
}

/// One thing a learned ability did in a turn, drawn as a line under the
/// answer (doc 93 §9.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityEvent {
    /// `used`, `failed`, `learned`, `improved`, `waiting`, `offered` or
    /// `turned_off`.
    pub event: &'static str,
    pub skill: String,
    /// Which version, for one learned or improved.
    pub version: Option<u32>,
}

impl AbilityEvent {
    fn new(event: &'static str, skill: &str, version: Option<u32>) -> Self {
        Self {
            event,
            skill: skill.to_string(),
            version,
        }
    }

    /// The line in plain words, for a surface that cannot draw it.
    pub fn words(&self) -> String {
        let skill = &self.skill;
        match (self.event, self.version) {
            ("used", _) => format!("Used {skill}"),
            ("failed", _) => format!("{skill} failed"),
            ("learned", _) => format!("Learned {skill}"),
            ("improved", Some(version)) => format!("Improved {skill} (version {version})"),
            ("improved", None) => format!("Improved {skill}"),
            ("waiting", _) => format!("{skill} is waiting for your review"),
            ("offered", _) => format!("{skill} is ready for every Agent"),
            _ => format!("{skill} was turned off"),
        }
    }

    /// The audit entry the projection draws.
    pub fn activity(&self) -> vak_session::ActivityRecord {
        let mut data = std::collections::BTreeMap::new();
        data.insert("event".to_string(), self.event.to_string());
        data.insert("skill".to_string(), self.skill.clone());
        if let Some(version) = self.version {
            data.insert("version".to_string(), version.to_string());
        }
        vak_session::ActivityRecord {
            activity_id: format!("ability-{}", uuid::Uuid::now_v7()),
            kind: vak_session::ActivityKind::Ability,
            status: if self.event == "failed" || self.event == "turned_off" {
                vak_session::ActivityStatus::Failed
            } else {
                vak_session::ActivityStatus::Succeeded
            },
            label: self.skill.clone(),
            detail: Some(self.words()),
            data,
        }
    }
}

/// What one call to an ability's program came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityUse {
    pub skill: String,
    pub digest: String,
    pub call: String,
    /// `None` when it answered; the fault when it failed.
    pub fault: Option<vak_control::Fault>,
}

/// What one turn did with the Agent's learned abilities.
#[derive(Debug, Clone, Default)]
pub struct TurnUses {
    pub uses: Vec<AbilityUse>,
    /// Every call the turn made, so a version made in this turn is told
    /// apart from one made earlier.
    pub calls: std::collections::BTreeSet<String>,
}

/// The calls of `turn` to the tools named in `programs` (skill name to
/// program digest) and how each ended, read from the turn's own entries.
pub fn uses_in_turn(
    session: &vak_session::SessionLog,
    turn: &str,
    programs: &std::collections::BTreeMap<String, String>,
) -> TurnUses {
    use vak_session::EntryPayload;
    let entries: Vec<_> = session
        .chain_to_root()
        .into_iter()
        .filter(|entry| entry.at_turn.as_deref() == Some(turn))
        .collect();
    let mut answered = std::collections::BTreeSet::new();
    let mut faults = std::collections::BTreeMap::new();
    let mut calls = Vec::new();
    let mut all = std::collections::BTreeSet::new();
    for entry in &entries {
        match &entry.payload {
            EntryPayload::Message(record) => {
                for block in &record.message.content {
                    if let vak_llm::ContentBlock::ToolUse { id, .. } = block {
                        all.insert(id.clone());
                    }
                    match block {
                        vak_llm::ContentBlock::ToolUse { id, name, .. }
                            if programs.contains_key(name) =>
                        {
                            calls.push((id.clone(), name.clone()));
                        }
                        vak_llm::ContentBlock::ToolResult {
                            tool_use_id,
                            is_error: false,
                            ..
                        } => {
                            answered.insert(tool_use_id.clone());
                        }
                        _ => {}
                    }
                }
            }
            EntryPayload::CallEffect(record) => {
                if let vak_session::types::CallEffect::Failed { fault } = record.effect {
                    faults.insert(record.tool_use_id.clone(), fault);
                }
            }
            _ => {}
        }
    }
    let uses = calls
        .into_iter()
        .filter_map(|(call, skill)| {
            let fault = faults.get(&call).copied();
            (fault.is_some() || answered.contains(&call)).then(|| AbilityUse {
                digest: programs.get(&skill).cloned().unwrap_or_default(),
                skill,
                call,
                fault,
            })
        })
        .collect();
    TurnUses { uses, calls: all }
}

/// One of an Agent's learned abilities as a person sees it (doc 93 §9.3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AbilityView {
    pub name: String,
    pub description: String,
    /// `working`, `failing`, `offered` (ready for every Agent), `waiting`
    /// (for the owner's review) or `turned_off`.
    pub state: &'static str,
    /// What it reaches, in the owner's words (`Program::reaches`).
    pub reaches: Vec<String>,
    /// Why it waits, for one waiting for review.
    pub why: Option<String>,
    /// The proposal to approve or decline, for one waiting.
    pub proposal: Option<String>,
    /// Its Library artifact, where its versions are.
    pub artifact: Option<String>,
    pub versions: usize,
}

fn offered_key(skill: &str, digest: &str) -> String {
    format!("ability-offered|{skill}|{digest}")
}

/// The inbox key of the entry that says proposal `id` is waiting.
pub fn waiting_key(id: &str) -> String {
    format!("proposal-waiting|{id}")
}

/// Marks read the entry that said proposal `id` was waiting, once the owner
/// approved or declined it.
fn settle_waiting(scope: &vak_config::scope::AgentScope, id: &str) {
    settle_key(scope, &waiting_key(id));
}

fn settle_key(scope: &vak_config::scope::AgentScope, key: &str) {
    for entry in crate::inbox::unread(scope) {
        if entry.dedupe_key.as_deref() == Some(key) {
            let _ = crate::inbox::ack(scope, &entry.id);
        }
    }
}

impl Core {
    fn ability_path(&self, skill: &str) -> std::path::PathBuf {
        self.scope().ability_standing().join(skill)
    }

    /// What the Agent's learned ability `skill` has done, if it has a record.
    pub fn ability_record(&self, skill: &str) -> Option<AbilityRecord> {
        vak_session::documents::read(&self.ability_path(skill))
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str(&text).ok())
    }

    /// Marks the Agent's learned ability `skill` withdrawn, or not, in its
    /// record.
    pub(crate) fn mark_withdrawn(&self, skill: &str, withdrawn: bool) -> Result<(), String> {
        vak_session::documents::update(&self.ability_path(skill), |current| {
            let mut record: AbilityRecord = current
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default();
            record.withdrawn = withdrawn;
            let text = serde_json::to_string(&record).map_err(|error| error.to_string())?;
            Ok(Some((text, ())))
        })?;
        Ok(())
    }

    pub(crate) fn record_ability_version(
        &self,
        skill: &str,
        version: VersionRecord,
    ) -> Result<(), String> {
        vak_session::documents::update(&self.ability_path(skill), |current| {
            let mut record: AbilityRecord = current
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default();
            record.versions.push(version.clone());
            record.withdrawn = false;
            let text = serde_json::to_string(&record).map_err(|error| error.to_string())?;
            Ok(Some((text, ())))
        })?;
        Ok(())
    }

    /// Takes what a turn's calls to the Agent's learned abilities came to
    /// into their records, then acts on R2 (doc 93): a version that has
    /// shown it works is offered to every Agent, and a fix that fails as
    /// its parent did is withdrawn for this Agent. Only a failure by the
    /// program's own fault counts; a refusal of the call's arguments or a
    /// stopped call does not.
    pub fn settle_ability_uses(&self, turn: &TurnUses) -> Vec<AbilityEvent> {
        let mut events = Vec::new();
        for found in &turn.uses {
            let path = self.ability_path(&found.skill);
            let verdict = vak_session::documents::update(&path, |current| {
                let Some(mut record) =
                    current.and_then(|text| serde_json::from_str::<AbilityRecord>(text).ok())
                else {
                    return Ok(None);
                };
                if record.withdrawn {
                    return Ok(None);
                }
                let Some(at) = record
                    .versions
                    .iter()
                    .rposition(|version| version.digest == found.digest)
                else {
                    return Ok(None);
                };
                let before = record.versions[at].clone();
                let current_version = &mut record.versions[at];
                match found.fault {
                    None if !current_version
                        .built_by
                        .as_ref()
                        .is_some_and(|call| turn.calls.contains(call)) =>
                    {
                        current_version.succeeded_elsewhere = true;
                    }
                    Some(vak_control::Fault::Tool) if current_version.failed.is_none() => {
                        current_version.failed = Some(found.call.clone());
                    }
                    _ => {}
                }
                let parent = current_version.parent.clone().and_then(|digest| {
                    record.versions[..at]
                        .iter()
                        .rev()
                        .find(|version| version.digest == digest)
                        .map(VersionRecord::facts)
                });
                let verdict = vak_judge::ability::verdict(record.versions[at].facts(), parent);
                match verdict {
                    vak_judge::ability::AbilityVerdict::Withdraw => record.withdrawn = true,
                    vak_judge::ability::AbilityVerdict::Offer if !record.versions[at].offered => {
                        record.versions[at].offered = true;
                    }
                    _ if record.versions[at] == before => return Ok(None),
                    _ => {}
                }
                let acted = match verdict {
                    vak_judge::ability::AbilityVerdict::Withdraw => Some(verdict),
                    vak_judge::ability::AbilityVerdict::Offer if !before.offered => Some(verdict),
                    _ => None,
                };
                let text = serde_json::to_string(&record).map_err(|error| error.to_string())?;
                Ok(Some((text, acted)))
            });
            match verdict {
                Ok(Some(Some(vak_judge::ability::AbilityVerdict::Withdraw))) => {
                    self.withdraw_ability(&found.skill);
                    events.push(AbilityEvent::new("turned_off", &found.skill, None));
                }
                Ok(Some(Some(vak_judge::ability::AbilityVerdict::Offer))) => {
                    events.push(AbilityEvent::new("offered", &found.skill, None));
                    let _ = crate::inbox::record_about_ability(
                        &self.shared_scope().as_agent(),
                        crate::inbox::Kind::AbilityOffered,
                        &format!("{} works: offer it to every Agent?", found.skill),
                        "It passed its tests, worked in a later conversation and has not failed since it was made.",
                        None,
                        &found.skill,
                        &offered_key(&found.skill, &found.digest),
                        None,
                    );
                }
                Ok(_) => {}
                Err(_) => tracing::warn!("an ability's record could not be updated"),
            }
        }
        events
    }

    /// What the Agent's learned abilities did in `turn`, one event each
    /// (doc 93 §9.1): used or failed by its calls, learned or improved by a
    /// version its call installed, waiting for a proposal its call made,
    /// and offered or turned off by the settlement. Read from records and
    /// call ids, never from any words.
    pub fn ability_events(&self, turn: &TurnUses) -> Vec<AbilityEvent> {
        let mut events = Vec::new();
        for found in &turn.uses {
            let event = match found.fault {
                None => "used",
                Some(vak_control::Fault::Tool) => "failed",
                Some(_) => continue,
            };
            let next = AbilityEvent::new(event, &found.skill, None);
            if !events.contains(&next) {
                events.push(next);
            }
        }
        if turn.calls.is_empty() {
            return events;
        }
        for path in vak_session::documents::under(&self.scope().ability_standing()) {
            let Some(skill) = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            let Some(record) = self.ability_record(&skill) else {
                continue;
            };
            let library = self.skill_artifact(&skill);
            for (at, version) in record.versions.iter().enumerate() {
                if version
                    .built_by
                    .as_ref()
                    .is_some_and(|call| turn.calls.contains(call))
                {
                    // The Library's number, as the ability's sheet shows it.
                    let place = library
                        .as_ref()
                        .and_then(|artifact| {
                            artifact
                                .versions
                                .iter()
                                .position(|known| known.id.to_string() == version.version)
                        })
                        .unwrap_or(at);
                    let number = u32::try_from(place + 1).unwrap_or(u32::MAX);
                    // Learned when it replaced nothing the Agent had.
                    events.push(if version.parent.is_none() {
                        AbilityEvent::new("learned", &skill, Some(number))
                    } else {
                        AbilityEvent::new("improved", &skill, Some(number))
                    });
                }
            }
        }
        for proposal in crate::learning::list_proposals(&self.scope(), self.cwd()) {
            if proposal
                .program
                .as_ref()
                .and_then(|review| review.call.as_ref())
                .is_some_and(|call| turn.calls.contains(call))
            {
                events.push(AbilityEvent::new("waiting", &proposal.name, None));
            }
        }
        events
    }

    fn withdraw_ability(&self, skill: &str) {
        let installed = self.scope().skills().join(skill);
        // The Library keeps every version; only the Agent's installed copy
        // goes, so its next turn no longer offers it.
        if std::fs::remove_dir_all(&installed).is_err() {
            tracing::warn!("a withdrawn ability's folder could not be removed");
        }
        let _ = crate::inbox::record_about_ability(
            &self.shared_scope().as_agent(),
            crate::inbox::Kind::AbilityWithdrawn,
            &format!("{skill} was turned off"),
            "It failed, and the fix proposed for it failed too. Its versions stay in the Library.",
            None,
            skill,
            &format!("ability-withdrawn|{skill}"),
            None,
        );
    }

    /// Installs the Agent's current version of `skill` for every Agent (the
    /// Shared layer), when its record has offered it (R2).
    pub fn promote_ability(&self, skill: &str) -> Result<(), String> {
        let record = self
            .ability_record(skill)
            .ok_or_else(|| format!("no learned ability '{skill}'"))?;
        let current = record
            .versions
            .last()
            .filter(|version| version.offered && !record.withdrawn)
            .ok_or_else(|| format!("'{skill}' has not shown yet that it works"))?;
        let installed = self.scope().skills().join(skill);
        let program = Program::load(&installed)?
            .ok_or_else(|| format!("'{skill}' is not installed for this Agent"))?;
        if program.digest != current.digest {
            return Err(format!("'{skill}' changed since it was offered"));
        }
        let bytes = vak_tools::capability::bundle(&installed)?;
        install(&bytes, &self.shared_skills_dir().join(skill))?;
        settle_key(
            &self.shared_scope().as_agent(),
            &offered_key(skill, &current.digest),
        );
        Ok(())
    }

    /// The Agent's learned abilities: installed, turned off, and waiting
    /// for review in any folder, one row each.
    pub fn abilities(&self) -> Vec<AbilityView> {
        let library = self.artifacts().list();
        let artifact_of = |name: &str| {
            library
                .iter()
                .find(|artifact| {
                    artifact.kind == crate::artifacts::ArtifactKind::Skill
                        && artifact.title.as_deref() == Some(name)
                })
                .map(|artifact| (artifact.id.to_string(), artifact.versions.len()))
        };
        let mut rows = Vec::new();
        let installed = self.scope().skills();
        let mut names: Vec<_> = std::fs::read_dir(&installed)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default();
        names.sort();
        for dir in names {
            let skill_md = dir.join("SKILL.md");
            let Some(skill) = std::fs::read_to_string(&skill_md)
                .ok()
                .and_then(|text| crate::skills::parse_text(&text, &skill_md))
            else {
                continue;
            };
            let record = self.ability_record(&skill.name);
            let current = record.as_ref().and_then(|record| record.versions.last());
            let state = match current {
                Some(version) if version.failed.is_some() => "failing",
                Some(version) if version.offered => "offered",
                _ => "working",
            };
            let (artifact, versions) = artifact_of(&skill.name).unzip();
            rows.push(AbilityView {
                reaches: Program::load(&dir)
                    .ok()
                    .flatten()
                    .map(|program| program.reaches())
                    .unwrap_or_default(),
                name: skill.name,
                description: skill.description,
                state,
                why: None,
                proposal: None,
                artifact,
                versions: versions.unwrap_or(1),
            });
        }
        for path in vak_session::documents::under(&self.scope().ability_standing()) {
            let Some(name) = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            if rows.iter().any(|row| row.name == name)
                || !self
                    .ability_record(&name)
                    .is_some_and(|record| record.withdrawn)
            {
                continue;
            }
            let (artifact, versions) = artifact_of(&name).unzip();
            // Installed nowhere: what it does and reaches are its newest
            // Library version's.
            let known = library
                .iter()
                .find(|known| Some(known.id.to_string()) == artifact);
            let description = known
                .and_then(|known| known.summary.clone())
                .unwrap_or_default();
            let reaches = known
                .and_then(|known| {
                    let newest = known.versions.last()?;
                    self.artifacts().bytes(known, &newest.id).ok()
                })
                .map(|bytes| crate::ability_sheet::reaches_of(&bytes))
                .unwrap_or_default();
            rows.push(AbilityView {
                name,
                description,
                state: "turned_off",
                reaches,
                why: None,
                proposal: None,
                artifact,
                versions: versions.unwrap_or(0),
            });
        }
        for proposal in crate::learning::list_proposals(&self.scope(), self.cwd()) {
            let review = proposal.program;
            let (artifact, versions) = review
                .as_ref()
                .map(|review| review.artifact.clone())
                .or_else(|| artifact_of(&proposal.name).map(|(id, _)| id))
                .map(|id| {
                    let versions = library
                        .iter()
                        .find(|artifact| artifact.id.to_string() == id)
                        .map_or(0, |artifact| artifact.versions.len());
                    (Some(id), versions)
                })
                .unwrap_or((None, 0));
            rows.push(AbilityView {
                name: proposal.name,
                description: proposal.description,
                state: "waiting",
                reaches: review
                    .as_ref()
                    .map(|review| review.reaches.clone())
                    .unwrap_or_default(),
                why: review.map(|review| review.why),
                proposal: Some(proposal.id),
                artifact,
                versions,
            });
        }
        rows
    }

    /// Approves proposal `id` for this Agent alone (doc 93 §9.2): its
    /// version is installed among the Agent's own skills, replacing the
    /// one it improves, and its record starts there.
    pub fn approve_for_agent(&self, id: &str) -> Result<String, String> {
        let proposal = crate::learning::list_proposals(&self.scope(), self.cwd())
            .into_iter()
            .find(|proposal| proposal.id == id)
            .ok_or_else(|| format!("no proposal '{id}'"))?;
        let review = proposal.program.clone().ok_or_else(|| {
            format!(
                "'{}' has no program; approve it for every Agent",
                proposal.name
            )
        })?;
        let target = self.scope().skills().join(&proposal.name);
        let parent = Program::load(&target).ok().flatten().map(|old| old.digest);
        let name = crate::learning::promote(
            &self.scope(),
            self.cwd(),
            id,
            &self.scope().skills(),
            Some(&self.artifacts()),
        )?;
        let program = Program::load(&target)?
            .ok_or_else(|| format!("'{name}' did not install with its program"))?;
        self.record_ability_version(
            &name,
            VersionRecord {
                digest: program.digest,
                version: review.version,
                parent,
                passed: true,
                ..VersionRecord::default()
            },
        )?;
        self.proposal_settled(id);
        Ok(name)
    }

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
                call: Some(proposal.call.to_string()),
            };
            let body = format!(
                "{skill_md}\n<!-- proposed-by: {} at {}; proposal id {id}; library: {artifact} {version} -->\n{}\n",
                proposal.session,
                chrono::Utc::now().to_rfc3339(),
                review.trailer()
            );
            vak_session::documents::create(&dir.join(format!("{id}.md")), &body)?;
            let _ = crate::inbox::record_about_ability(
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
                name,
                &waiting_key(&id),
                Some(&trace),
            );
            Standing::Review { id, why }
        } else {
            let target = self.scope().skills().join(name);
            let parent = Program::load(&target).ok().flatten().map(|old| old.digest);
            install(&bytes, &target)?;
            self.record_ability_version(
                name,
                VersionRecord {
                    digest: program.digest.clone(),
                    version: version.to_string(),
                    parent,
                    built_by: Some(proposal.call.to_string()),
                    passed: true,
                    ..VersionRecord::default()
                },
            )?;
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
