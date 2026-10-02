#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use vak_core::state::{self, Root};
use vak_mail_calendar::connection_ledger::ConnectionLedger;

#[test]
fn agent_connection_ledger_is_covered_by_the_durable_state_registry() {
    vak_config::paths::isolate_home_for_tests();
    let ledger = ConnectionLedger::for_agent("registry-agent").unwrap();
    let data_home = vak_config::paths::data_home();
    let relative = ledger.path().strip_prefix(data_home).unwrap();

    assert_eq!(
        relative,
        Path::new("agents/registry-agent/mail-calendar/connections.jsonl")
    );
    assert!(
        state::is_declared(Root::Data, relative),
        "the connection ledger must be included in backup, purge, and upgrade checks"
    );
}
