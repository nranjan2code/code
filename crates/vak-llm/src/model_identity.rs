//! Which ids at different services name the same model
//! (docs/design/15-reliability.md, "Same model at other services").
//!
//! One model has a different id at each service that sells it: a bare name
//! at its maker, `vendor/name` at an aggregator, `region.vendor.name-v1:0`
//! on Bedrock. The route ladder stands one in for another only when a person
//! has confirmed they are the same model (`[route] same_model`). This module
//! holds that vocabulary and the suggestion that proposes groups from live
//! catalogues. A suggestion is never used until it is confirmed, because a
//! wrong match would quietly serve a different model.
//!
//! Nothing here names a model or a vendor (invariant 9): the suggestion reads
//! how ids are spelled, never what any model is.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One model as one service names it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

impl ModelRef {
    /// Read the config spelling `provider/model`. The provider is the part
    /// before the FIRST `/`: provider names never contain one, while model
    /// ids often do (`openrouter/anthropic/claude-…`). This is the spelling
    /// `[intent] classify_model` already uses for a provider-qualified model.
    pub fn parse(spelling: &str) -> Option<Self> {
        let (provider, model) = spelling.trim().split_once('/')?;
        let (provider, model) = (provider.trim(), model.trim());
        (!provider.is_empty() && !model.is_empty()).then(|| Self {
            provider: provider.to_string(),
            model: model.to_string(),
        })
    }

    /// The config spelling; [`ModelRef::parse`] reads it back unchanged.
    pub fn spelling(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }

    fn is(&self, provider: &str, model: &str) -> bool {
        self.provider == provider && self.model == model
    }
}

/// Merge groups that share a member, drop repeated members and any group
/// left with fewer than two, and sort, so one set of facts has one spelling
/// whichever layer or confirmation each came from.
pub fn merge_groups(groups: impl IntoIterator<Item = Vec<ModelRef>>) -> Vec<Vec<ModelRef>> {
    // Existing sets stay pairwise disjoint, so absorbing every set the new
    // one touches in a single pass is a complete union.
    let mut merged: Vec<BTreeSet<ModelRef>> = Vec::new();
    for group in groups {
        let mut set: BTreeSet<ModelRef> = group.into_iter().collect();
        let mut index = 0;
        while index < merged.len() {
            if merged[index].iter().any(|member| set.contains(member)) {
                set.extend(merged.swap_remove(index));
            } else {
                index += 1;
            }
        }
        merged.push(set);
    }
    let mut out: Vec<Vec<ModelRef>> = merged
        .into_iter()
        .filter(|set| set.len() >= 2)
        .map(|set| set.into_iter().collect())
        .collect();
    out.sort();
    out
}

/// Every confirmed stand-in for `provider`/`model`: the members of each
/// group that contains it, without the model itself.
pub fn group_mates(groups: &[Vec<ModelRef>], provider: &str, model: &str) -> Vec<ModelRef> {
    let mut mates = BTreeSet::new();
    for group in groups {
        if group.iter().any(|member| member.is(provider, model)) {
            mates.extend(
                group
                    .iter()
                    .filter(|member| !member.is(provider, model))
                    .cloned(),
            );
        }
    }
    mates.into_iter().collect()
}

/// What an id says about its model once the service's own spelling is
/// removed: `base` names the model, and `snapshot` the dated release when
/// the id pins one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdentityKey {
    pub base: String,
    pub snapshot: Option<String>,
}

/// Read the parts of an id that name the model, dropping the parts that
/// belong to one service's catalogue: a `vendor/` path, `region.vendor.`
/// prefixes, a `-v1:0` revision, a `-latest` alias, and `.` or `_` in place
/// of `-`. A date the id pins is kept apart, because two dated releases of
/// one name are two models.
pub fn identity_key(model: &str) -> Option<IdentityKey> {
    let lowered = model.trim().to_ascii_lowercase();
    let mut id = lowered.rsplit('/').next().unwrap_or_default();
    // Letter-only dot segments are catalogue prefixes (`us.`, `anthropic.`);
    // a segment holding a digit or a dash is already the model's own name
    // (`gpt-4.1`, `llama3.1`), so stripping stops there.
    while let Some((head, tail)) = id.split_once('.') {
        if head.is_empty() || tail.is_empty() || !head.bytes().all(|b| b.is_ascii_lowercase()) {
            break;
        }
        id = tail;
    }
    let mut id = id.to_string();
    strip_revision(&mut id);
    if let Some(alias) = id.strip_suffix("-latest") {
        id = alias.to_string();
    }
    let snapshot = take_snapshot(&mut id);
    let base = normalise_separators(&id);
    (!base.is_empty()).then_some(IdentityKey { base, snapshot })
}

/// Drop a trailing `-v<major>:<minor>` catalogue revision. A bare `-v3` is
/// kept: it is part of names like `deepseek-v3`.
fn strip_revision(id: &mut String) {
    let Some(at) = id.rfind("-v") else {
        return;
    };
    let Some((major, minor)) = id[at + 2..].split_once(':') else {
        return;
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if at > 0 && digits(major) && digits(minor) {
        id.truncate(at);
    }
}

/// Take a trailing release date: `-YYYYMMDD`, `@YYYYMMDD` or `-YYYY-MM-DD`.
fn take_snapshot(id: &mut String) -> Option<String> {
    if !id.is_ascii() {
        return None;
    }
    let bytes = id.as_bytes();
    let digit_run =
        |from: usize, len: usize| bytes[from..from + len].iter().all(u8::is_ascii_digit);
    let len = bytes.len();
    if len > 9 && matches!(bytes[len - 9], b'-' | b'@') && digit_run(len - 8, 8) {
        let snapshot = id[len - 8..].to_string();
        id.truncate(len - 9);
        return Some(snapshot);
    }
    if len > 11
        && bytes[len - 11] == b'-'
        && bytes[len - 6] == b'-'
        && bytes[len - 3] == b'-'
        && digit_run(len - 10, 4)
        && digit_run(len - 5, 2)
        && digit_run(len - 2, 2)
    {
        let snapshot = format!(
            "{}{}{}",
            &id[len - 10..len - 6],
            &id[len - 5..len - 3],
            &id[len - 2..]
        );
        id.truncate(len - 11);
        return Some(snapshot);
    }
    None
}

/// `.` and `_` read as `-`, and a `-` between a letter and a digit is
/// dropped, so `llama-3.1`, `llama3-1` and `llama3.1` read alike. A `:` tag
/// is kept: it selects a local variant, which is not the hosted model.
fn normalise_separators(id: &str) -> String {
    let chars: Vec<char> = id
        .chars()
        .map(|c| if c == '.' || c == '_' { '-' } else { c })
        .collect();
    let mut out = String::with_capacity(chars.len());
    for (index, &c) in chars.iter().enumerate() {
        let between_letter_and_digit = c == '-'
            && index > 0
            && chars[index - 1].is_ascii_alphabetic()
            && chars.get(index + 1).is_some_and(char::is_ascii_digit);
        if !between_letter_and_digit {
            out.push(c);
        }
    }
    out
}

/// One service's model list, as discovery returned it.
#[derive(Debug, Clone, Copy)]
pub struct Catalogue<'a> {
    pub provider: &'a str,
    /// The account that answers for this provider. Two provider names over
    /// one account (one key, two wire protocols) are one service, and a
    /// match between them is not a second place to reach the model.
    pub service: &'a str,
    pub models: &'a [String],
}

/// Propose groups of ids that name one model at two or more services.
///
/// Ids are matched on their [`identity_key`]. When one name has several
/// dated releases, each release is its own group and the undated aliases
/// form another, since which release an alias points at is unknown. A
/// proposal is only ever shown for confirmation; nothing uses it until a
/// person agrees.
pub fn suggest_groups(catalogues: &[Catalogue<'_>]) -> Vec<Vec<ModelRef>> {
    type Member<'a> = (ModelRef, &'a str, Option<String>);
    let mut by_base: BTreeMap<String, Vec<Member<'_>>> = BTreeMap::new();
    for catalogue in catalogues {
        for model in catalogue.models {
            let Some(key) = identity_key(model) else {
                continue;
            };
            by_base.entry(key.base).or_default().push((
                ModelRef {
                    provider: catalogue.provider.to_string(),
                    model: model.clone(),
                },
                catalogue.service,
                key.snapshot,
            ));
        }
    }
    let mut out = Vec::new();
    for members in by_base.into_values() {
        let mut by_snapshot: BTreeMap<Option<String>, Vec<&Member<'_>>> = BTreeMap::new();
        for member in &members {
            by_snapshot
                .entry(member.2.clone())
                .or_default()
                .push(member);
        }
        let dated = by_snapshot
            .keys()
            .filter(|snapshot| snapshot.is_some())
            .count();
        let partitions: Vec<Vec<&Member<'_>>> = if dated <= 1 {
            vec![members.iter().collect()]
        } else {
            by_snapshot.into_values().collect()
        };
        for partition in partitions {
            let services: BTreeSet<&str> = partition.iter().map(|member| member.1).collect();
            if services.len() < 2 {
                continue;
            }
            let group: BTreeSet<ModelRef> =
                partition.iter().map(|member| member.0.clone()).collect();
            out.push(group.into_iter().collect::<Vec<_>>());
        }
    }
    out.sort();
    out
}

/// Narrow proposals to what a person has not already confirmed.
///
/// Without a focus, a proposal one confirmed group already holds whole is
/// dropped. With a focus (the model being routed), only proposals naming it
/// are kept, each trimmed to the focus plus the members not yet confirmed
/// as its stand-ins, and one left with nothing new is dropped. Confirming a
/// trimmed proposal still joins the full group: groups sharing a member
/// merge.
pub fn unconfirmed(
    proposals: Vec<Vec<ModelRef>>,
    confirmed: &[Vec<ModelRef>],
    focus: Option<&ModelRef>,
) -> Vec<Vec<ModelRef>> {
    proposals
        .into_iter()
        .filter_map(|group| match focus {
            None => (!confirmed
                .iter()
                .any(|known| group.iter().all(|member| known.contains(member))))
            .then_some(group),
            Some(focus) => {
                if !group.contains(focus) {
                    return None;
                }
                let mates = group_mates(confirmed, &focus.provider, &focus.model);
                let fresh: Vec<ModelRef> = group
                    .into_iter()
                    .filter(|member| member == focus || !mates.contains(member))
                    .collect();
                (fresh.len() >= 2).then_some(fresh)
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn r(provider: &str, model: &str) -> ModelRef {
        ModelRef {
            provider: provider.into(),
            model: model.into(),
        }
    }

    fn key(model: &str) -> (String, Option<String>) {
        let key = identity_key(model).unwrap();
        (key.base, key.snapshot)
    }

    #[test]
    fn spelling_splits_at_the_first_slash_and_round_trips() {
        let parsed = ModelRef::parse("openrouter/anthropic/claude-x.5").unwrap();
        assert_eq!(parsed, r("openrouter", "anthropic/claude-x.5"));
        assert_eq!(ModelRef::parse(&parsed.spelling()), Some(parsed));
        let bedrock = ModelRef::parse("bedrock/us.vendor.model-v1:0").unwrap();
        assert_eq!(bedrock.model, "us.vendor.model-v1:0");
        for bad in ["", "no-slash", "/model", "provider/", " / "] {
            assert_eq!(ModelRef::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn service_spellings_of_one_model_share_a_key() {
        let maker = key("claude-sonnet-4-5");
        assert_eq!(key("anthropic/claude-sonnet-4.5"), maker);
        assert_eq!(key("claude-sonnet-4-5@20250929").0, maker.0);
        assert_eq!(
            key("us.anthropic.claude-sonnet-4-5-20250929-v1:0"),
            (maker.0.clone(), Some("20250929".into()))
        );
        assert_eq!(
            key("meta.llama3-1-8b-instruct-v1:0"),
            key("meta-llama/llama-3.1-8b-instruct")
        );
        assert_eq!(
            key("gpt-4o-2024-08-06"),
            ("gpt4o".into(), Some("20240806".into()))
        );
        assert_eq!(key("vendor/model-latest"), key("model"));
        assert_eq!(key("models/gemini-2.5-flash"), key("gemini-2.5-flash"));
    }

    #[test]
    fn different_models_keep_different_keys() {
        assert_ne!(key("gpt-4o"), key("gpt-4o-mini"));
        assert_ne!(key("deepseek-v3"), key("deepseek-v2"));
        assert_eq!(key("deepseek-v3").0, "deepseek-v3", "a bare -v3 is a name");
        assert_ne!(key("gpt-4.1"), key("gpt-4"));
        assert_ne!(
            key("llama3.1:8b"),
            key("meta-llama/llama-3.1-8b"),
            "a local tag is not the hosted model"
        );
        assert_eq!(identity_key("   "), None);
        assert_eq!(identity_key("ünïcode-20250101").unwrap().snapshot, None);
    }

    #[test]
    fn merge_unions_overlapping_groups_and_drops_singletons() {
        let groups = vec![
            vec![r("a", "x"), r("b", "x2")],
            vec![r("c", "x3"), r("b", "x2")],
            vec![r("d", "y"), r("d", "y")],
            vec![r("e", "z"), r("f", "z")],
        ];
        assert_eq!(
            merge_groups(groups),
            vec![
                vec![r("a", "x"), r("b", "x2"), r("c", "x3")],
                vec![r("e", "z"), r("f", "z")],
            ]
        );
    }

    #[test]
    fn mates_are_every_group_member_but_the_model_itself() {
        let groups = merge_groups([
            vec![r("a", "x"), r("b", "x2")],
            vec![r("e", "z"), r("f", "z")],
        ]);
        assert_eq!(group_mates(&groups, "a", "x"), vec![r("b", "x2")]);
        assert!(group_mates(&groups, "a", "z").is_empty());
    }

    #[test]
    fn suggestions_need_two_services_not_two_provider_names() {
        let direct = vec!["gpt-5".to_string(), "gpt-5-mini".to_string()];
        let aggregator = vec!["openai/gpt-5".to_string(), "other/thing".to_string()];
        let one_account = [
            Catalogue {
                provider: "openai",
                service: "key-1",
                models: &direct,
            },
            Catalogue {
                provider: "openai-responses",
                service: "key-1",
                models: &direct,
            },
        ];
        assert!(
            suggest_groups(&one_account).is_empty(),
            "two wire protocols over one key are one service"
        );

        let mut catalogues = one_account.to_vec();
        catalogues.push(Catalogue {
            provider: "openrouter",
            service: "key-2",
            models: &aggregator,
        });
        assert_eq!(
            suggest_groups(&catalogues),
            vec![vec![
                r("openai", "gpt-5"),
                r("openai-responses", "gpt-5"),
                r("openrouter", "openai/gpt-5"),
            ]]
        );
    }

    #[test]
    fn proposals_carry_only_what_is_not_yet_confirmed() {
        let proposal = vec![r("a", "x"), r("b", "x"), r("c", "vendor/x"), r("d", "x")];
        let confirmed = vec![vec![r("a", "x"), r("b", "x"), r("c", "vendor/x")]];

        assert_eq!(
            unconfirmed(vec![proposal.clone()], &confirmed, Some(&r("a", "x"))),
            vec![vec![r("a", "x"), r("d", "x")]],
            "trimmed to the focus and the one name not yet confirmed"
        );
        assert!(
            unconfirmed(vec![proposal.clone()], &confirmed, Some(&r("e", "y"))).is_empty(),
            "a proposal that does not name the focus is not shown"
        );
        let whole = vec![r("a", "x"), r("b", "x")];
        assert!(
            unconfirmed(vec![whole.clone()], &confirmed, Some(&r("a", "x"))).is_empty(),
            "nothing new, nothing to confirm"
        );
        assert!(unconfirmed(vec![whole], &confirmed, None).is_empty());
        assert_eq!(
            unconfirmed(vec![proposal.clone()], &confirmed, None),
            vec![proposal],
            "without a focus a partly new proposal is shown whole"
        );
    }

    #[test]
    fn several_dated_releases_split_and_aliases_stand_apart() {
        let maker = vec![
            "model-x".to_string(),
            "model-x-20240101".to_string(),
            "model-x-20250101".to_string(),
        ];
        let host = vec![
            "vendor/model-x".to_string(),
            "us.vendor.model-x-20250101-v1:0".to_string(),
        ];
        let catalogues = [
            Catalogue {
                provider: "maker",
                service: "m",
                models: &maker,
            },
            Catalogue {
                provider: "host",
                service: "h",
                models: &host,
            },
        ];
        assert_eq!(
            suggest_groups(&catalogues),
            vec![
                vec![
                    r("host", "us.vendor.model-x-20250101-v1:0"),
                    r("maker", "model-x-20250101")
                ],
                vec![r("host", "vendor/model-x"), r("maker", "model-x")],
            ]
        );

        let single_release = vec!["model-y-20250101".to_string(), "model-y".to_string()];
        let aggregator = vec!["vendor/model-y".to_string()];
        let catalogues = [
            Catalogue {
                provider: "maker",
                service: "m",
                models: &single_release,
            },
            Catalogue {
                provider: "host",
                service: "h",
                models: &aggregator,
            },
        ];
        assert_eq!(
            suggest_groups(&catalogues),
            vec![vec![
                r("host", "vendor/model-y"),
                r("maker", "model-y"),
                r("maker", "model-y-20250101"),
            ]],
            "with one dated release the undated alias joins it"
        );
    }
}
