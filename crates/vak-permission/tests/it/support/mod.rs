//! What the built-in tools declare to the engine (`Tool::permission`), as
//! test data: the engine knows no tool by name. `vak-core`'s
//! `built_in_tools_declare_what_the_permission_tests_assume` checks this
//! table against the real tools.

#![allow(dead_code)]

use vak_permission::{Declared, PermissionEngine, Reach, Subject};

pub fn built_in() -> Vec<(String, Declared)> {
    let path = |required| Subject::Path {
        argument: "path",
        required,
    };
    [
        ("read", Declared::new(Reach::ReadPath, path(true))),
        ("doc_read", Declared::new(Reach::ReadPath, path(false))),
        ("glob", Declared::new(Reach::ReadPath, path(false))),
        ("grep", Declared::new(Reach::ReadPath, path(false))),
        ("ls", Declared::new(Reach::ReadPath, path(false))),
        ("write", Declared::new(Reach::WritePath, path(false))),
        ("edit", Declared::new(Reach::WritePath, path(false))),
        ("office_apply", Declared::new(Reach::WritePath, path(false))),
        (
            "bash",
            Declared::new(Reach::Command, Subject::Command("command")),
        ),
        ("recall", Declared::reach(Reach::ReadStore)),
        (
            "skill",
            Declared::new(Reach::ReadStore, Subject::Value("name")),
        ),
        ("commitments", Declared::reach(Reach::ReadStore)),
        ("mail_calendar", Declared::reach(Reach::ReadStore)),
        ("remember", Declared::reach(Reach::Learning)),
        ("forget_memory", Declared::reach(Reach::Learning)),
        ("propose_skill", Declared::reach(Reach::Learning)),
        ("webfetch", Declared::reach(Reach::Network)),
        ("mail_calendar_send", Declared::reach(Reach::Network)),
        (
            "mail_calendar_event_create",
            Declared::reach(Reach::Network),
        ),
        (
            "mail_calendar_event_update",
            Declared::reach(Reach::Network),
        ),
        (
            "mail_calendar_event_cancel",
            Declared::reach(Reach::Network),
        ),
        (
            "mail_calendar_event_reconcile",
            Declared::reach(Reach::Network),
        ),
        (
            "mail_calendar_mail_reconcile",
            Declared::reach(Reach::Network),
        ),
        ("ask_parent", Declared::reach(Reach::Coordination)),
        ("workers", Declared::reach(Reach::Coordination)),
        ("emit_table_card", Declared::reach(Reach::Display)),
        (
            "mcp",
            Declared::new(Reach::Integration, Subject::IntegrationCall),
        ),
        ("task", Declared::new(Reach::Other, Subject::Value("label"))),
    ]
    .into_iter()
    .map(|(name, declared)| (name.to_string(), declared))
    .collect()
}

/// An engine from rule strings, told what the built-in tools declare.
pub fn engine(specs: &[String]) -> Result<PermissionEngine, vak_permission::RuleError> {
    PermissionEngine::from_rule_strings(specs).map(|engine| engine.with_tools(built_in()))
}
