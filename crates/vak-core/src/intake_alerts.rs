//! Intake alerts (plan M6.5, docs/design/76-intake-and-knowledge.md §7):
//! which new items reach a person. An alert is a Document `alerts/<alr>`
//! owned by an Agent, matching items of its sources by keyword, tag or
//! source. Only items that reach the Agent are matched: an item detection
//! held is matched once a person releases it, never before.
//!
//! What an alert has matched is a Document `alert-state/<alr>`, moved by
//! CAS: each item matches an alert once, ever, and matches found while
//! the alert's cooldown runs wait in it until the next evaluation after
//! the cooldown, so nothing is sent twice and nothing is dropped.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vak_config::scope::SharedScope;
use vak_session::ids::{AlertId, PrincipalId, SourceId};

use crate::intake::{IntakeError, Item, Source};

/// How many item ids an alert remembers having matched.
const MATCHED_KEPT: usize = 4096;
/// The most items one alert holds waiting; older ones are sent first.
const PENDING_KEPT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub id: AlertId,
    pub name: String,
    /// The Agent whose sources it watches.
    pub agent: String,
    /// Any one of these words or phrases, case-insensitively.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    /// Items of a source carrying any of these tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Items of these sources; empty is every source of the Agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceId>,
    /// The least time between two notices of this alert.
    #[serde(default)]
    pub cooldown_minutes: u64,
    /// A channel to send its notices to as well as the inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliver_to: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<PrincipalId>,
}

impl Alert {
    pub fn validate(&self) -> Result<(), IntakeError> {
        if self.name.trim().is_empty() || self.name.chars().count() > 120 {
            return Err(IntakeError::Invalid(
                "an alert's name is 1 to 120 characters".into(),
            ));
        }
        if self.keywords.is_empty() && self.tags.is_empty() && self.sources.is_empty() {
            return Err(IntakeError::Invalid(
                "an alert names keywords, tags or sources to match".into(),
            ));
        }
        if self.keywords.len() > 50 || self.keywords.iter().any(|k| k.trim().is_empty()) {
            return Err(IntakeError::Invalid(
                "an alert has at most 50 non-empty keywords".into(),
            ));
        }
        if self.cooldown_minutes > 7 * 24 * 60 {
            return Err(IntakeError::Invalid(
                "an alert's cooldown is at most 7 days".into(),
            ));
        }
        Ok(())
    }

    /// Whether `item` of `source` is one this alert is about.
    pub fn matches(&self, source: &Source, item: &Item) -> bool {
        if source.agent != self.agent {
            return false;
        }
        if !self.sources.is_empty() && !self.sources.contains(&source.id) {
            return false;
        }
        if !self.tags.is_empty() && !self.tags.iter().any(|tag| source.tags.contains(tag)) {
            return false;
        }
        if self.keywords.is_empty() {
            return true;
        }
        let haystack = format!("{}\n{}", item.title, item.text).to_lowercase();
        self.keywords
            .iter()
            .any(|keyword| haystack.contains(&keyword.trim().to_lowercase()))
    }
}

/// One matched item, as a notice names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Matched {
    pub item: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_sent: Option<DateTime<Utc>>,
    #[serde(default)]
    pending: Vec<Matched>,
    #[serde(default)]
    matched: Vec<String>,
}

/// An item that reached its Agent, to be matched.
pub struct Candidate<'a> {
    pub source: &'a Source,
    pub item_id: String,
    pub item: &'a Item,
}

/// A notice an alert owes now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub alert: Alert,
    pub items: Vec<Matched>,
}

fn alert_path(shared: &SharedScope, id: &str) -> PathBuf {
    shared.alerts().join(id)
}

fn decode(name: &str, text: &str) -> Result<Alert, IntakeError> {
    serde_json::from_str(text).map_err(|error| IntakeError::Corrupt(name.into(), error.to_string()))
}

pub fn list(shared: &SharedScope) -> Result<Vec<Alert>, IntakeError> {
    let mut alerts = Vec::new();
    for path in vak_session::documents::under(&shared.alerts()) {
        let name = path.display().to_string();
        if let Some(text) = vak_session::documents::read(&path).map_err(IntakeError::Store)? {
            alerts.push(decode(&name, &text)?);
        }
    }
    alerts.sort_by_key(|alert| alert.created_at);
    Ok(alerts)
}

pub fn get(shared: &SharedScope, id: &str) -> Result<Option<Alert>, IntakeError> {
    match vak_session::documents::read(&alert_path(shared, id)).map_err(IntakeError::Store)? {
        Some(text) => decode(id, &text).map(Some),
        None => Ok(None),
    }
}

pub fn create(shared: &SharedScope, alert: &Alert) -> Result<(), IntakeError> {
    alert.validate()?;
    let text =
        serde_json::to_string(alert).map_err(|error| IntakeError::Store(error.to_string()))?;
    vak_session::documents::create(&alert_path(shared, &alert.id.to_string()), &text)
        .map_err(IntakeError::Store)
}

/// Applies `change`; an alert's id, Agent and creation never change.
pub fn update(
    shared: &SharedScope,
    id: &str,
    mut change: impl FnMut(&mut Alert),
) -> Result<Alert, IntakeError> {
    let mut failure: Option<IntakeError> = None;
    let saved = vak_session::documents::update(&alert_path(shared, id), |current| {
        let Some(text) = current else {
            failure = Some(IntakeError::NotFound(id.into()));
            return Ok(None);
        };
        let mut alert = match decode(id, text) {
            Ok(alert) => alert,
            Err(error) => {
                failure = Some(error);
                return Ok(None);
            }
        };
        let fixed = (alert.id, alert.agent.clone(), alert.created_at);
        change(&mut alert);
        alert.id = fixed.0;
        alert.agent = fixed.1.clone();
        alert.created_at = fixed.2;
        if let Err(error) = alert.validate() {
            failure = Some(error);
            return Ok(None);
        }
        let text = serde_json::to_string(&alert).map_err(|error| error.to_string())?;
        Ok(Some((text, alert)))
    })
    .map_err(IntakeError::Store)?;
    match (saved, failure) {
        (Some(alert), _) => Ok(alert),
        (None, Some(error)) => Err(error),
        (None, None) => Err(IntakeError::NotFound(id.into())),
    }
}

pub fn delete(shared: &SharedScope, id: &str) -> Result<bool, IntakeError> {
    let removed =
        vak_session::documents::forget(&alert_path(shared, id)).map_err(IntakeError::Store)?;
    let _ = vak_session::documents::forget(&shared.alert_state().join(id));
    Ok(removed)
}

/// Matches `candidates` against every enabled alert and returns the
/// notices owed at `now`: each item matches an alert once, and an alert
/// in its cooldown keeps its matches for a later evaluation. Pass no
/// candidates to send only what waited out a cooldown.
pub fn evaluate(
    shared: &SharedScope,
    candidates: &[Candidate<'_>],
    now: DateTime<Utc>,
) -> Result<Vec<Due>, IntakeError> {
    let mut dues = Vec::new();
    for alert in list(shared)?.into_iter().filter(|alert| alert.enabled) {
        let fresh: Vec<Matched> = candidates
            .iter()
            .filter(|candidate| alert.matches(candidate.source, candidate.item))
            .map(|candidate| Matched {
                item: candidate.item_id.clone(),
                title: candidate.item.title.clone(),
                link: candidate.item.link.clone(),
            })
            .collect();
        let path = shared.alert_state().join(alert.id.to_string());
        let mut sending: Vec<Matched> = Vec::new();
        let cooldown = Duration::minutes(alert.cooldown_minutes as i64);
        vak_session::documents::update(&path, |current| {
            let mut state: State = current
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default();
            let mut changed = false;
            for matched in &fresh {
                if !state.matched.contains(&matched.item) {
                    state.matched.push(matched.item.clone());
                    state.pending.push(matched.clone());
                    changed = true;
                }
            }
            let excess = state.matched.len().saturating_sub(MATCHED_KEPT);
            state.matched.drain(..excess);
            let excess = state.pending.len().saturating_sub(PENDING_KEPT);
            state.pending.drain(..excess);
            let rested = state.last_sent.is_none_or(|last| now - last >= cooldown);
            sending = Vec::new();
            if rested && !state.pending.is_empty() {
                sending = std::mem::take(&mut state.pending);
                state.last_sent = Some(now);
                changed = true;
            }
            if !changed {
                return Ok(None);
            }
            let text = serde_json::to_string(&state).map_err(|error| error.to_string())?;
            Ok(Some((text, ())))
        })
        .map_err(IntakeError::Store)?;
        if !sending.is_empty() {
            dues.push(Due {
                alert,
                items: sending,
            });
        }
    }
    Ok(dues)
}

/// A notice's title and text, plain and short.
pub fn notice(due: &Due) -> (String, String) {
    let count = due.items.len();
    let title = format!(
        "{}: {count} new {}",
        due.alert.name,
        if count == 1 { "item" } else { "items" }
    );
    let body = due
        .items
        .iter()
        .map(|matched| match &matched.link {
            Some(link) => format!("- {} ({link})", matched.title),
            None => format!("- {}", matched.title),
        })
        .collect::<Vec<_>>()
        .join("\n");
    (title, body)
}
