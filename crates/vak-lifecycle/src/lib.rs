//! The lifecycle reconciler's decisions (plan M7a,
//! docs/design/74-lifecycle-and-data-administration.md §3.1, §5): retention
//! labels, and the plan they produce over what was observed. Pure: no
//! clock, no filesystem and no vak dependency, so the same observation
//! always gives the same plan. Observing a data home and committing a
//! plan are the caller's (`vak_core::lifecycle`); this crate decides.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// What a retention rule applies to. The names are the rows of the
/// default label (doc 74 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    Execution,
    Environment,
    Checkpoint,
    Trash,
    DraftVersion,
    DocumentHistory,
    InboxEntry,
    AllowlistEntry,
    ActivitySegment,
    Incident,
    Run,
    Telemetry,
}

impl DataClass {
    /// Every class, in the order a plan acts on them (doc 74 §5): the
    /// cheapest and most rebuildable first, records last.
    pub const ORDER: [DataClass; 12] = [
        DataClass::Execution,
        DataClass::Environment,
        DataClass::Telemetry,
        DataClass::Checkpoint,
        DataClass::Trash,
        DataClass::DraftVersion,
        DataClass::DocumentHistory,
        DataClass::InboxEntry,
        DataClass::AllowlistEntry,
        DataClass::Run,
        DataClass::ActivitySegment,
        DataClass::Incident,
    ];

    fn rank(self) -> usize {
        Self::ORDER
            .iter()
            .position(|class| *class == self)
            .unwrap_or(usize::MAX)
    }
}

/// What happens to an item when its rule says its time is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExpiry {
    /// Remove it for good.
    Remove,
    /// Move it to the trash, whose own window then applies.
    Trash,
}

/// One row of a label: how long a class is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub class: DataClass,
    /// Seconds after an item's `since` at which it expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_after_secs: Option<i64>,
    /// The most the class may hold; the oldest items go first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    pub on_expiry: OnExpiry,
}

/// A retention label: a named set of rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    pub rules: Vec<Rule>,
}

const DAY: i64 = 24 * 60 * 60;

impl Label {
    /// The tenant's default label (doc 74 §3.1). Conversation records and
    /// their objects have no rule: they are kept.
    pub fn default_tenant() -> Self {
        let after = |class, days: i64, on_expiry| Rule {
            class,
            delete_after_secs: Some(days * DAY),
            max_bytes: None,
            on_expiry,
        };
        Label {
            id: "default".into(),
            name: "Default".into(),
            rules: vec![
                after(DataClass::Trash, 30, OnExpiry::Remove),
                after(DataClass::Run, 180, OnExpiry::Remove),
                after(DataClass::Checkpoint, 30, OnExpiry::Remove),
                after(DataClass::Environment, 7, OnExpiry::Remove),
                after(DataClass::DraftVersion, 60, OnExpiry::Trash),
                after(DataClass::DocumentHistory, 90, OnExpiry::Remove),
                after(DataClass::InboxEntry, 90, OnExpiry::Remove),
                after(DataClass::AllowlistEntry, 90, OnExpiry::Remove),
                after(DataClass::ActivitySegment, 400, OnExpiry::Remove),
                after(DataClass::Incident, 400, OnExpiry::Remove),
                Rule {
                    class: DataClass::Telemetry,
                    delete_after_secs: Some(14 * DAY),
                    max_bytes: Some(200 * 1024 * 1024),
                    on_expiry: OnExpiry::Remove,
                },
            ],
        }
    }

    pub fn rule(&self, class: DataClass) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.class == class)
    }
}

/// Why an item is not acted on although its rule has expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Guard {
    /// A legal hold covers it.
    Held,
    /// Work that uses it is still running.
    Live,
    /// A person kept it: accepted, saved, starred or shared.
    Kept,
}

/// One thing the reconciler observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// Stable for the item's life; an id, never a name or a path.
    pub id: String,
    pub class: DataClass,
    /// When its retention clock started (it settled, was made, was written).
    pub since: DateTime<Utc>,
    pub bytes: u64,
    pub files: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<Guard>,
}

/// Why an action is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Older than the rule's `delete_after`.
    Age,
    /// The class holds more than the rule's `max_bytes`.
    Size,
}

/// One transition the plan would make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    /// Names this transition of this item and nothing else, so committing
    /// it twice is committing it once.
    pub key: String,
    pub class: DataClass,
    pub item: String,
    pub does: OnExpiry,
    pub reason: Reason,
    /// When the rule expired the item (`Age`), else when it was observed.
    pub due: DateTime<Utc>,
    pub bytes: u64,
    pub files: u64,
}

/// An expired item the plan leaves alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guarded {
    pub class: DataClass,
    pub item: String,
    pub guard: Guard,
}

/// What a class holds and what the plan would take from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassSummary {
    pub class: DataClass,
    pub items: u64,
    pub bytes: u64,
    pub due: u64,
    pub due_bytes: u64,
    pub guarded: u64,
}

/// The dry-run: what the reconciler would do now, and what it would not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub at: DateTime<Utc>,
    pub label: String,
    pub actions: Vec<Action>,
    pub guarded: Vec<Guarded>,
    pub classes: Vec<ClassSummary>,
    /// Classes the label has a rule for and nothing observed: no plan can
    /// be made for them, which is not the same as nothing being due.
    pub unobserved: Vec<DataClass>,
    pub reclaimable_bytes: u64,
}

fn action(item: &Item, rule: &Rule, reason: Reason, due: DateTime<Utc>) -> Action {
    let does = match rule.on_expiry {
        OnExpiry::Remove => "remove",
        OnExpiry::Trash => "trash",
    };
    Action {
        key: format!("{does}:{}", item.id),
        class: item.class,
        item: item.id.clone(),
        does: rule.on_expiry,
        reason,
        due,
        bytes: item.bytes,
        files: item.files,
    }
}

/// The plan for `items` under `label` at `now`. `observed` names the
/// classes an observer looked at, whether or not it found anything.
/// Deterministic: the same inputs give the same plan, in the same order.
pub fn plan(items: &[Item], observed: &[DataClass], label: &Label, now: DateTime<Utc>) -> Plan {
    let mut actions = Vec::new();
    let mut guarded = Vec::new();
    let mut classes = Vec::new();
    for class in DataClass::ORDER {
        let Some(rule) = label.rule(class) else {
            continue;
        };
        let mut of_class: Vec<&Item> = items.iter().filter(|item| item.class == class).collect();
        of_class.sort_by(|a, b| a.since.cmp(&b.since).then_with(|| a.id.cmp(&b.id)));
        let mut summary = ClassSummary {
            class,
            items: of_class.len() as u64,
            bytes: of_class.iter().map(|item| item.bytes).sum(),
            due: 0,
            due_bytes: 0,
            guarded: 0,
        };
        let mut over = rule
            .max_bytes
            .map_or(0, |max| summary.bytes.saturating_sub(max));
        for item in of_class {
            let expired = rule
                .delete_after_secs
                .map(|secs| item.since + Duration::seconds(secs))
                .filter(|due| *due <= now);
            let found = match expired {
                Some(due) => Some((Reason::Age, due)),
                None if over > 0 => Some((Reason::Size, now)),
                None => None,
            };
            let Some((reason, due)) = found else {
                continue;
            };
            if let Some(guard) = item.guard {
                summary.guarded += 1;
                guarded.push(Guarded {
                    class,
                    item: item.id.clone(),
                    guard,
                });
                continue;
            }
            over = over.saturating_sub(item.bytes);
            summary.due += 1;
            summary.due_bytes += item.bytes;
            actions.push(action(item, rule, reason, due));
        }
        classes.push(summary);
    }
    actions.sort_by(|a: &Action, b: &Action| {
        a.class
            .rank()
            .cmp(&b.class.rank())
            .then_with(|| a.due.cmp(&b.due))
            .then_with(|| a.key.cmp(&b.key))
    });
    let unobserved = DataClass::ORDER
        .into_iter()
        .filter(|class| label.rule(*class).is_some() && !observed.contains(class))
        .collect();
    Plan {
        at: now,
        label: label.id.clone(),
        reclaimable_bytes: actions.iter().map(|action| action.bytes).sum(),
        actions,
        guarded,
        classes,
        unobserved,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn at(day: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_800_000_000 + day * DAY, 0).unwrap()
    }

    fn item(id: &str, class: DataClass, day: i64, bytes: u64) -> Item {
        Item {
            id: id.into(),
            class,
            since: at(day),
            bytes,
            files: 1,
            guard: None,
        }
    }

    const ALL: &[DataClass] = &DataClass::ORDER;

    #[test]
    fn reconciler_is_idempotent() {
        let mut items = vec![
            item("env-b", DataClass::Environment, 0, 10),
            item("env-a", DataClass::Environment, 0, 10),
            item("draft-1", DataClass::DraftVersion, 0, 5),
            item("log-1", DataClass::Telemetry, 80, 7),
        ];
        let label = Label::default_tenant();
        let first = plan(&items, ALL, &label, at(100));
        // The same observation, in any order, is the same plan.
        items.reverse();
        assert_eq!(plan(&items, ALL, &label, at(100)), first);
        // Every action names one transition of one item.
        let mut keys: Vec<&str> = first.actions.iter().map(|a| a.key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), first.actions.len());
        // What a commit would take away is not planned again.
        let gone: Vec<&str> = first.actions.iter().map(|a| a.item.as_str()).collect();
        let left: Vec<Item> = items
            .into_iter()
            .filter(|item| !gone.contains(&item.id.as_str()))
            .collect();
        assert!(plan(&left, ALL, &label, at(100)).actions.is_empty());
    }

    #[test]
    fn an_item_expires_exactly_when_its_rule_says() {
        let label = Label::default_tenant();
        let items = [item("env-1", DataClass::Environment, 0, 10)];
        assert!(plan(&items, ALL, &label, at(6)).actions.is_empty());
        let due = plan(&items, ALL, &label, at(7));
        assert_eq!(due.actions.len(), 1);
        assert_eq!(due.actions[0].key, "remove:env-1");
        assert_eq!(due.actions[0].reason, Reason::Age);
        assert_eq!(due.actions[0].due, at(7));
        assert_eq!(due.reclaimable_bytes, 10);
    }

    #[test]
    fn a_draft_goes_to_the_trash_and_a_kept_one_stays() {
        let label = Label::default_tenant();
        let mut kept = item("draft-kept", DataClass::DraftVersion, 0, 5);
        kept.guard = Some(Guard::Kept);
        let items = [item("draft-old", DataClass::DraftVersion, 0, 5), kept];
        let made = plan(&items, ALL, &label, at(61));
        assert_eq!(made.actions.len(), 1);
        assert_eq!(made.actions[0].does, OnExpiry::Trash);
        assert_eq!(made.actions[0].item, "draft-old");
        assert_eq!(
            made.guarded,
            [Guarded {
                class: DataClass::DraftVersion,
                item: "draft-kept".into(),
                guard: Guard::Kept,
            }]
        );
    }

    #[test]
    fn a_class_over_its_size_loses_its_oldest_first() {
        let label = Label::default_tenant();
        let mib = 1024 * 1024;
        let items = [
            item("log-new", DataClass::Telemetry, 99, 90 * mib),
            item("log-old", DataClass::Telemetry, 97, 90 * mib),
            item("log-mid", DataClass::Telemetry, 98, 90 * mib),
        ];
        let made = plan(&items, ALL, &label, at(100));
        let taken: Vec<&str> = made.actions.iter().map(|a| a.item.as_str()).collect();
        assert_eq!(taken, ["log-old"], "270 MiB held, 200 allowed");
        assert_eq!(made.actions[0].reason, Reason::Size);
    }

    #[test]
    fn a_plan_acts_on_rebuildable_classes_before_records() {
        let label = Label::default_tenant();
        let items = [
            item("run-1", DataClass::Run, 0, 1),
            item("env-1", DataClass::Environment, 0, 1),
            item("log-1", DataClass::Telemetry, 0, 1),
        ];
        let order: Vec<DataClass> = plan(&items, ALL, &label, at(400))
            .actions
            .iter()
            .map(|a| a.class)
            .collect();
        assert_eq!(
            order,
            [DataClass::Environment, DataClass::Telemetry, DataClass::Run]
        );
    }

    #[test]
    fn a_class_nobody_looked_at_is_named_not_assumed_empty() {
        let label = Label::default_tenant();
        let made = plan(&[], &[DataClass::Environment], &label, at(0));
        assert!(!made.unobserved.contains(&DataClass::Environment));
        assert!(made.unobserved.contains(&DataClass::Trash));
        assert!(
            !made.unobserved.contains(&DataClass::Execution),
            "no rule, no gap"
        );
    }
}
