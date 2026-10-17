//! An Agent's review of its failing abilities (docs/design/93 §8): an
//! automation it owns, off until the owner turns it on, that looks at the
//! learned abilities whose version in use failed since it last looked and,
//! when there are some, runs a turn that proposes fixes. Everything that
//! turn proposes waits for review, because nobody watched it (R7).

use crate::Core;
use crate::skill_programs::AbilityRecord;
use crate::triggers::{self, Schedule, Trigger, TriggerAction, TriggerKind};

/// One ability whose version in use failed and has not been reviewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failing {
    pub skill: String,
    /// The version's Library number, when the Library still has it.
    pub version: Option<usize>,
    /// The arguments of the call that failed, as written.
    pub arguments: Option<String>,
    /// What the failed call answered.
    pub answer: Option<String>,
}

/// When a new review runs: each night, in the owner's time zone. The owner
/// changes it on the Automations page like any automation's.
const NIGHTLY: &str = "0 3 * * *";

/// The review's name, as the Automations page shows it.
pub const REVIEW_NAME: &str = "Review abilities that failed";

impl Core {
    fn review_agent(&self) -> String {
        self.agent_identity()
            .map_or_else(|| crate::vak_agent_identity().id, |agent| agent.id.clone())
    }

    /// This Agent's review of failures, if it was ever turned on.
    pub fn ability_review(&self) -> Option<Trigger> {
        let agent = self.review_agent();
        triggers::list(&self.shared_scope())
            .ok()?
            .into_iter()
            .find(|trigger| trigger.reviews_abilities() && trigger.agent == agent)
    }

    /// Turns this Agent's review of failures on or off. Off pauses it, so
    /// its runs stay; on resumes it, or makes it the first time.
    pub fn set_ability_review(&self, on: bool) -> Result<Trigger, String> {
        if let Some(existing) = self.ability_review() {
            return triggers::update(&self.shared_scope(), &existing.id.to_string(), |trigger| {
                trigger.enabled = on;
                Ok(())
            })
            .map_err(|error| error.to_string());
        }
        if !on {
            return Err("this Agent's review of failures was never turned on".into());
        }
        let mut trigger = Trigger {
            id: vak_session::ids::TriggerId::new(),
            name: REVIEW_NAME.into(),
            agent: self.review_agent(),
            agent_revision: None,
            space: vak_config::spaces::key(self.cwd()),
            enabled: true,
            kind: TriggerKind::Schedule {
                schedule: Schedule::Cron {
                    expr: NIGHTLY.into(),
                    timezone: None,
                },
            },
            action: TriggerAction::AbilityReview,
            deliver_to: None,
            on_crash: triggers::OnCrash::Skip,
            scope: None,
            created_at: chrono::Utc::now(),
            created_by: None,
            home: None,
            made_by: Some(triggers::MadeBy::Owner),
            page: None,
        };
        trigger.in_zone(&self.owner_timezone());
        triggers::create(&self.shared_scope(), &trigger).map_err(|error| error.to_string())?;
        Ok(trigger)
    }

    /// The Agent's learned abilities whose version in use failed and that
    /// no review has taken yet, with the failed call's arguments and answer
    /// found through the catalog.
    pub fn failing_abilities(&self) -> Vec<Failing> {
        let catalog = self.catalog().ok();
        if let Some(catalog) = &catalog {
            let _ = catalog.catch_up();
        }
        let mut found = Vec::new();
        for path in vak_session::documents::under(&self.scope().ability_standing()) {
            let Some(skill) = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            let Some(record) = self
                .ability_record(&skill)
                .filter(|record| !record.withdrawn)
            else {
                continue;
            };
            let Some(current) = record.versions.last() else {
                continue;
            };
            let Some(call) = current.failed.as_ref().filter(|_| !current.reviewed) else {
                continue;
            };
            let evidence = catalog.as_ref().and_then(|catalog| {
                let node = catalog
                    .calls_of(&skill, None, &self.catalog_audience())
                    .ok()?
                    .into_iter()
                    .find(|node| node.id.ends_with(&format!(":{call}")))?;
                catalog.call_evidence(&node.id).ok().flatten()
            });
            let (arguments, answer) = evidence.unzip();
            found.push(Failing {
                version: self.skill_artifact(&skill).and_then(|artifact| {
                    artifact
                        .versions
                        .iter()
                        .position(|known| known.id.to_string() == current.version)
                        .map(|at| at + 1)
                }),
                skill,
                arguments,
                answer,
            });
        }
        found.sort_by(|a, b| a.skill.cmp(&b.skill));
        found
    }

    /// Marks the version in use of each of `skills` as given to a review.
    pub fn mark_reviewed(&self, skills: &[String]) {
        for skill in skills {
            let path = self.scope().ability_standing().join(skill);
            let marked = vak_session::documents::update(&path, |current| {
                let Some(mut record) =
                    current.and_then(|text| serde_json::from_str::<AbilityRecord>(text).ok())
                else {
                    return Ok(None);
                };
                let Some(last) = record.versions.last_mut() else {
                    return Ok(None);
                };
                last.reviewed = true;
                let text = serde_json::to_string(&record).map_err(|error| error.to_string())?;
                Ok(Some((text, ())))
            });
            if marked.is_err() {
                tracing::warn!("an ability's review mark was not written");
            }
        }
    }

    /// Offers the owner, once, to turn on this Agent's review of failures,
    /// when one of its abilities fails and nothing reviews them.
    pub(crate) fn offer_ability_review(&self, skill: &str) {
        if self.ability_review().is_some() {
            return;
        }
        let agent = self.review_agent();
        let _ = crate::inbox::record_about_ability(
            &self.shared_scope().as_agent(),
            crate::inbox::Kind::AbilityReviewOffered,
            &format!("{skill} failed: review failures each night?"),
            "The Agent can look at its abilities that failed each night and propose fixes; every fix waits for you. Turn it on in Settings, Capabilities, Learned.",
            None,
            skill,
            &format!("ability-review-offered|{agent}"),
            None,
        );
    }
}

/// The review turn's prompt: each failing ability with its failed call,
/// and what to do. Nothing in it is read back to decide anything.
pub fn brief(failing: &[Failing]) -> String {
    let mut text = String::from(
        "This is your review of your learned abilities that failed. Nobody is watching this run, so whatever you propose waits for the owner's review.\n",
    );
    for found in failing {
        text.push_str(&format!(
            "\n- {}{} failed.",
            found.skill,
            found
                .version
                .map(|number| format!(" (version {number})"))
                .unwrap_or_default()
        ));
        if let Some(arguments) = &found.arguments {
            text.push_str(&format!("\n  It was called with: {arguments}"));
        }
        if let Some(answer) = &found.answer {
            text.push_str(&format!("\n  It answered: {}", answer.trim()));
        }
    }
    text.push_str(
        "\n\nFor each one, load the skill to see its program and tests. If the program is wrong, write a fixed copy with a test for this case and propose it again under the same name with propose_skill. If the call itself was wrong and the program is right, change nothing and say so. End with one line per ability saying what you did.",
    );
    text
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_brief_names_each_failure_with_its_call() {
        let text = brief(&[Failing {
            skill: "split-bill".into(),
            version: Some(3),
            arguments: Some(r#"{"amount":10,"people":0}"#.into()),
            answer: Some("the program exited with 1".into()),
        }]);
        assert!(text.contains("split-bill (version 3) failed."));
        assert!(text.contains(r#"called with: {"amount":10,"people":0}"#));
        assert!(text.contains("It answered: the program exited with 1"));
        assert!(text.contains("propose_skill"));
    }
}
