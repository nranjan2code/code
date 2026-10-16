//! One learned ability as its sheet shows it, and what the owner does from
//! there (docs/design/93-self-evolving-capabilities.md §9.1, §9.2): its
//! versions with what each changed and how its tests went, its uses since
//! the version in use, using an earlier version, and turning it off.

use crate::Core;
use crate::artifacts::{Artifact, ArtifactKind, ArtifactStep, NewVersion, VersionSource};
use crate::skill_programs::{AbilityView, CaseResult, VersionRecord, install};
use vak_session::ids::{PrincipalId, VersionId};
use vak_tools::capability::{Program, bundle_digest, bundle_files};

/// A file a version added, changed or removed against the version before.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileChange {
    pub path: String,
    /// `added`, `changed` or `removed`.
    pub change: &'static str,
    /// Which part of the skill it is: `instructions`, `declaration`,
    /// `tests`, `presentation` or `program`.
    pub part: &'static str,
}

/// Which part of a skill a file is, from the skill's own layout.
fn part_of(path: &str) -> &'static str {
    let first = path.split('/').next().unwrap_or(path);
    if path == "SKILL.md" {
        "instructions"
    } else if path == vak_tools::capability::DECLARATION {
        "declaration"
    } else if first == vak_tools::capability::TESTS_DIR && path.contains('/') {
        "tests"
    } else if first == "presentation" && path.contains('/') {
        "presentation"
    } else {
        "program"
    }
}

/// A version's tests as the runtime ran them.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SheetTests {
    pub passed: bool,
    pub run: String,
    /// Each case, when the run's report is kept where this Agent reads.
    pub cases: Vec<CaseResult>,
}

/// One Library version of an ability.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SheetVersion {
    /// Its place in the Library's history, from 1.
    pub number: usize,
    pub version: String,
    pub at: chrono::DateTime<chrono::Utc>,
    /// `in_use`, `waiting` (for the owner's review), `failed_tests`,
    /// `failed_in_use` (a call failed by its own fault), `earlier` (in use
    /// once) or `not_used`.
    pub state: &'static str,
    /// Made by the owner choosing it, not by an Agent.
    pub by_you: bool,
    pub changes: Vec<FileChange>,
    pub tests: Option<SheetTests>,
    /// The owner may put it in use: its tests passed and it is neither in
    /// use nor waiting.
    pub can_use: bool,
    pub digest: String,
}

/// One call to the ability.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SheetUse {
    pub at: Option<String>,
    /// The conversation it was made in, as its plain id.
    pub conversation: Option<String>,
    pub title: Option<String>,
    /// `done` or `failed`; absent while it has no result.
    pub outcome: Option<String>,
}

/// The capability sheet (doc 93 §9.1).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AbilitySheet {
    #[serde(flatten)]
    pub ability: AbilityView,
    /// Its versions, newest first.
    pub history: Vec<SheetVersion>,
    /// Calls since the version in use was made, newest first.
    pub uses: Vec<SheetUse>,
    /// The version `uses` counts from.
    pub uses_since: Option<usize>,
    /// The conversation a version waiting for review came from.
    pub proposed_in: Option<String>,
    /// The declaration of the version in use, for technical details.
    pub declaration: Option<String>,
}

fn changes(before: Option<&[u8]>, after: &[u8]) -> Vec<FileChange> {
    let after = bundle_files(after).unwrap_or_default();
    let before = before
        .and_then(|bytes| bundle_files(bytes).ok())
        .unwrap_or_default();
    let mut found = Vec::new();
    for (path, text) in &after {
        match before.get(path) {
            None => found.push(FileChange {
                path: path.clone(),
                change: "added",
                part: part_of(path),
            }),
            Some(old) if old != text => found.push(FileChange {
                path: path.clone(),
                change: "changed",
                part: part_of(path),
            }),
            Some(_) => {}
        }
    }
    for path in before.keys().filter(|path| !after.contains_key(*path)) {
        found.push(FileChange {
            path: path.clone(),
            change: "removed",
            part: part_of(path),
        });
    }
    found
}

/// What the program in a skill bundle reaches, in the owner's words.
pub(crate) fn reaches_of(bytes: &[u8]) -> Vec<String> {
    let dir = vak_config::paths::runtime_dir()
        .join("ability-sheets")
        .join(uuid::Uuid::now_v7().simple().to_string());
    let reaches = vak_tools::capability::unbundle(bytes, &dir)
        .ok()
        .and_then(|()| Program::load(&dir).ok().flatten())
        .map(|program| program.reaches())
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    reaches
}

impl Core {
    /// The Library artifact holding the versions of the learned ability
    /// `name`.
    pub fn skill_artifact(&self, name: &str) -> Option<Artifact> {
        let home = self.ability_space();
        let mut found: Vec<Artifact> = self
            .artifacts()
            .list()
            .into_iter()
            .filter(|artifact| {
                artifact.kind == ArtifactKind::Skill && artifact.title.as_deref() == Some(name)
            })
            .collect();
        // Its home first; one recorded elsewhere by an earlier build, the
        // newest.
        found.sort_by_key(|artifact| (artifact.space == home, artifact.updated_at));
        found.pop()
    }

    /// The sheet of the Agent's learned ability `name`, if it has one.
    pub fn ability_sheet(&self, name: &str) -> Option<AbilitySheet> {
        let ability = self.abilities().into_iter().find(|row| row.name == name)?;
        let installed = self.scope().skills().join(name);
        let program = Program::load(&installed).ok().flatten();
        let record = self.ability_record(name).unwrap_or_default();
        let proposals = crate::learning::list_proposals(&self.scope());
        let waiting = proposals
            .iter()
            .find(|proposal| ability.proposal.as_deref() == Some(proposal.id.as_str()));
        let artifact = self.skill_artifact(name);
        let mut versions = Vec::new();
        let mut uses_since = None;
        let mut since_at = None;
        // The version in use is the one its record installed last; the same
        // bytes chosen again later are a version of their own.
        let in_use_digest = program
            .as_ref()
            .filter(|_| !record.withdrawn)
            .map(|program| program.digest.clone());
        let in_use = in_use_digest.as_ref().and_then(|digest| {
            record
                .versions
                .iter()
                .rev()
                .find(|known| &known.digest == digest)
                .map(|known| known.version.clone())
        });
        if let Some(artifact) = &artifact {
            let library = self.artifacts();
            let mut before: Option<Vec<u8>> = None;
            for (at, version) in artifact.versions.iter().enumerate() {
                let bytes = library.bytes(artifact, &version.id).ok();
                let digest = bytes
                    .as_deref()
                    .and_then(|bytes| bundle_digest(bytes).ok())
                    .unwrap_or_default();
                let in_record: Vec<&VersionRecord> = record
                    .versions
                    .iter()
                    .filter(|known| known.digest == digest)
                    .collect();
                let passed = version.verified.as_ref().is_some_and(|tests| tests.passed);
                let state = if in_use.as_deref() == Some(version.id.to_string().as_str()) {
                    "in_use"
                } else if waiting
                    .and_then(|proposal| proposal.program.as_ref())
                    .is_some_and(|review| review.version == version.id.to_string())
                {
                    "waiting"
                } else if version.verified.is_some() && !passed {
                    "failed_tests"
                } else if in_record.iter().any(|known| known.failed.is_some()) {
                    "failed_in_use"
                } else if !in_record.is_empty() {
                    "earlier"
                } else {
                    "not_used"
                };
                if state == "in_use" {
                    uses_since = Some(at + 1);
                    since_at = Some(version.at);
                }
                versions.push(SheetVersion {
                    number: at + 1,
                    version: version.id.to_string(),
                    at: version.at,
                    state,
                    by_you: matches!(version.source, VersionSource::Person),
                    changes: bytes
                        .as_deref()
                        .map(|after| changes(before.as_deref(), after))
                        .unwrap_or_default(),
                    tests: version.verified.as_ref().map(|tests| SheetTests {
                        passed: tests.passed,
                        run: tests.run.clone(),
                        cases: self
                            .skill_program_verification(&digest)
                            .map(|report| report.cases)
                            .unwrap_or_default(),
                    }),
                    can_use: passed
                        && !matches!(state, "in_use" | "waiting")
                        && in_use_digest.as_deref() != Some(digest.as_str()),
                    digest,
                });
                before = bytes;
            }
            versions.reverse();
        }
        let uses = self
            .catalog()
            .ok()
            .and_then(|catalog| {
                let _ = catalog.catch_up();
                let since = since_at.map(|at| at.to_rfc3339());
                catalog
                    .calls_of(name, since.as_deref(), &self.catalog_audience())
                    .ok()
                    .map(|calls| {
                        calls
                            .into_iter()
                            .map(|call| {
                                let session = call.session.clone();
                                SheetUse {
                                    at: call.created_at,
                                    title: session
                                        .as_deref()
                                        .and_then(|id| catalog.open_node(id).ok().flatten())
                                        .and_then(|node| node.title),
                                    conversation: session.map(|id| {
                                        id.strip_prefix("ses_").map(str::to_string).unwrap_or(id)
                                    }),
                                    outcome: call.status,
                                }
                            })
                            .collect()
                    })
            })
            .unwrap_or_default();
        Some(AbilitySheet {
            declaration: std::fs::read_to_string(
                installed.join(vak_tools::capability::DECLARATION),
            )
            .ok(),
            proposed_in: waiting
                .and_then(|proposal| proposal.derived_from.as_ref())
                .map(|from| from.conversation.clone()),
            ability,
            history: versions,
            uses,
            uses_since,
        })
    }

    /// Puts an earlier version of the learned ability `name` back in use
    /// for this Agent, as the owner's choice: its bytes become a new Library
    /// version made by the owner, whose parent is the newest, so nothing is
    /// rewritten and undoing it is choosing again. Only a version whose
    /// tests passed may be chosen. Returns the new version's number.
    pub fn use_ability_version(
        &self,
        name: &str,
        version: &str,
        actor: Option<PrincipalId>,
    ) -> Result<usize, String> {
        let artifact = self
            .skill_artifact(name)
            .ok_or_else(|| format!("'{name}' has no versions in the Library"))?;
        let id = VersionId::parse(version).map_err(|_| format!("no version '{version}'"))?;
        let chosen = artifact
            .versions
            .iter()
            .find(|known| known.id == id)
            .ok_or_else(|| format!("no version '{version}' of '{name}'"))?;
        let tests = chosen
            .verified
            .clone()
            .filter(|tests| tests.passed)
            .ok_or_else(|| "this version did not pass its tests".to_string())?;
        let library = self.artifacts();
        let bytes = library
            .bytes(&artifact, &id)
            .map_err(|error| error.to_string())?;
        let digest = bundle_digest(&bytes)?;
        let target = self.scope().skills().join(name);
        let current = Program::load(&target).ok().flatten();
        let withdrawn = self
            .ability_record(name)
            .is_some_and(|record| record.withdrawn);
        if !withdrawn
            && current
                .as_ref()
                .is_some_and(|program| program.digest == digest)
        {
            return Err("this version is the one in use".into());
        }
        let made = library
            .transaction(|tx| {
                let made = tx.version(
                    artifact.id,
                    NewVersion {
                        parent: None,
                        bytes: &bytes,
                        source: VersionSource::Person,
                    },
                    None,
                    actor,
                )?;
                tx.record(
                    artifact.id,
                    ArtifactStep::Verified {
                        version: made,
                        run: tests.run.clone(),
                        passed: true,
                    },
                    None,
                    actor,
                )?;
                Ok(made)
            })
            .map_err(|error| format!("the version was not recorded in the Library: {error}"))?;
        install(&bytes, &target)?;
        // The newest version chosen again is the same version: it is in use
        // once more, not installed as another.
        if self
            .ability_record(name)
            .and_then(|record| record.versions.last().cloned())
            .is_some_and(|last| last.version == made.to_string())
        {
            self.mark_withdrawn(name, false)?;
        } else {
            self.record_ability_version(
                name,
                VersionRecord {
                    digest,
                    version: made.to_string(),
                    parent: current.map(|program| program.digest),
                    passed: true,
                    ..VersionRecord::default()
                },
            )?;
        }
        Ok(self
            .skill_artifact(name)
            .and_then(|artifact| artifact.versions.iter().position(|known| known.id == made))
            .map_or(0, |at| at + 1))
    }

    /// Turns the learned ability `name` off for this Agent, as the owner's
    /// choice: its installed copy goes, so its next turn no longer offers
    /// it, and every version stays in the Library to be used again.
    pub fn turn_off_ability(&self, name: &str) -> Result<(), String> {
        let installed = self.scope().skills().join(name);
        if Program::load(&installed).ok().flatten().is_none() {
            return Err(format!("'{name}' is not in use for this Agent"));
        }
        self.mark_withdrawn(name, true)?;
        std::fs::remove_dir_all(&installed)
            .map_err(|error| format!("'{name}' could not be removed: {error}"))
    }
}

/// One installed version of an ability as the measure shows it (doc 93
/// §9.6): its calls and failures, and what R2 says of it now.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct VersionMeasure {
    /// Its place in the Library's history, when the Library still has it.
    pub number: Option<usize>,
    pub at: Option<chrono::DateTime<chrono::Utc>>,
    /// Calls while it was the version installed (from the start, when the
    /// Library no longer has the version), and how many failed.
    pub calls: usize,
    pub failed: usize,
    pub passed: bool,
    pub succeeded_elsewhere: bool,
    pub failed_in_use: bool,
    pub offered: bool,
    /// `offer`, `withdraw` or `keep`: `vak_judge::ability::verdict` on it.
    pub verdict: &'static str,
    /// The version installed now.
    pub current: bool,
}

impl Core {
    /// Each installed version of the learned ability `name`, oldest first,
    /// with the calls made while it was installed (found through the
    /// catalog) and R2's verdict on it.
    pub fn ability_measure(&self, name: &str) -> Vec<VersionMeasure> {
        let record = self.ability_record(name).unwrap_or_default();
        let artifact = self.skill_artifact(name);
        let placed = |version: &str| {
            artifact.as_ref().and_then(|artifact| {
                artifact
                    .versions
                    .iter()
                    .position(|known| known.id.to_string() == version)
                    .map(|at| (at + 1, artifact.versions[at].at))
            })
        };
        let calls: Vec<(chrono::DateTime<chrono::Utc>, bool)> = self
            .catalog()
            .ok()
            .and_then(|catalog| {
                let _ = catalog.catch_up();
                catalog.calls_of(name, None, &self.catalog_audience()).ok()
            })
            .unwrap_or_default()
            .into_iter()
            .filter_map(|call| {
                let at = chrono::DateTime::parse_from_rfc3339(call.created_at.as_deref()?).ok()?;
                Some((
                    at.with_timezone(&chrono::Utc),
                    call.status.as_deref() == Some("failed"),
                ))
            })
            .collect();
        let starts: Vec<_> = record
            .versions
            .iter()
            .map(|version| placed(&version.version).map(|(_, at)| at))
            .collect();
        let last = record.versions.len().saturating_sub(1);
        record
            .versions
            .iter()
            .enumerate()
            .map(|(at, version)| {
                let from = starts[at];
                let until = starts[at + 1..].iter().flatten().next().copied();
                let window: Vec<_> = calls
                    .iter()
                    .filter(|(when, _)| {
                        from.is_none_or(|from| *when >= from)
                            && until.is_none_or(|until| *when < until)
                    })
                    .collect();
                let parent = version.parent.as_ref().and_then(|digest| {
                    record.versions[..at]
                        .iter()
                        .rev()
                        .find(|earlier| &earlier.digest == digest)
                        .map(VersionRecord::facts)
                });
                VersionMeasure {
                    number: placed(&version.version).map(|(number, _)| number),
                    at: from,
                    calls: window.len(),
                    failed: window.iter().filter(|(_, failed)| *failed).count(),
                    passed: version.passed,
                    succeeded_elsewhere: version.succeeded_elsewhere,
                    failed_in_use: version.failed.is_some(),
                    offered: version.offered,
                    verdict: match vak_judge::ability::verdict(version.facts(), parent) {
                        vak_judge::ability::AbilityVerdict::Offer => "offer",
                        vak_judge::ability::AbilityVerdict::Withdraw => "withdraw",
                        vak_judge::ability::AbilityVerdict::Keep => "keep",
                    },
                    current: at == last && !record.withdrawn,
                }
            })
            .collect()
    }

    /// Takes back from every Agent the Shared copy of `name`, an ability an
    /// Agent of this Core made (its versions are in this Core's Library):
    /// the copy goes at once, so the next call to it is refused, and its
    /// versions stay. A Shared skill no Agent made is the owner's file and
    /// is never removed here.
    pub fn revoke_shared_ability(&self, name: &str) -> Result<(), String> {
        if self.skill_artifact(name).is_none() {
            return Err(format!("'{name}' was not made by this Agent"));
        }
        let shared = self.shared_skills_dir().join(name);
        if Program::load(&shared).ok().flatten().is_none() {
            return Err(format!("'{name}' is not given to every Agent"));
        }
        std::fs::remove_dir_all(&shared)
            .map_err(|error| format!("'{name}' could not be removed: {error}"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn bundle(files: &[(&str, &str)]) -> Vec<u8> {
        let files: std::collections::BTreeMap<_, _> = files.iter().copied().collect();
        serde_json::to_vec(&serde_json::json!({ "files": files })).unwrap()
    }

    #[test]
    fn a_version_says_which_files_it_added_changed_and_removed() {
        let before = bundle(&[("run.py", "a"), ("tests/one.json", "{}"), ("old.md", "x")]);
        let after = bundle(&[
            ("run.py", "b"),
            ("tests/one.json", "{}"),
            ("tests/two.json", "{}"),
        ]);
        let found = changes(Some(&before), &after);
        let words: Vec<_> = found
            .iter()
            .map(|change| format!("{} {}", change.change, change.path))
            .collect();
        assert_eq!(
            words,
            ["changed run.py", "added tests/two.json", "removed old.md"]
        );
        assert_eq!(changes(None, &after).len(), 3);
        let parts: Vec<_> = found.iter().map(|change| change.part).collect();
        assert_eq!(parts, ["program", "tests", "program"]);
        assert_eq!(part_of("SKILL.md"), "instructions");
        assert_eq!(part_of("capability.json"), "declaration");
        assert_eq!(part_of("presentation/table.json"), "presentation");
    }
}
