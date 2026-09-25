//! A session ledger's lock belongs to its handle alone. The server reopens
//! ledgers while other turns spawn tool workers, and a child being spawned
//! holds duplicates of the server's descriptors until it execs. A lock
//! released only by closing its descriptor stayed with such a child, so a
//! reopen failed with `SessionError::Locked` while nothing held the ledger.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use vak_session::SessionLog;
use vak_session::types::SessionError;
use vak_tools::ToolContext;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reopening_a_ledger_while_workers_spawn_never_finds_it_locked() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = dir.path().join("session.jsonl");
    std::fs::write(&ledger, "").unwrap();
    std::fs::write(dir.path().join("note.txt"), "hello").unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let reopening = std::thread::spawn({
        let stop = stop.clone();
        move || {
            let (mut reopens, mut locked) = (0u32, 0u32);
            while !stop.load(Ordering::Relaxed) {
                match SessionLog::open(ledger.clone()) {
                    Ok(log) => drop(log),
                    Err(SessionError::Locked(_)) => locked += 1,
                    Err(error) => panic!("reopen failed: {error}"),
                }
                reopens += 1;
            }
            (reopens, locked)
        }
    });

    let worker = PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker"));
    let read = vak_tools::brokered_default_tools(worker)
        .into_iter()
        .find(|tool| tool.name() == "read")
        .expect("read is a built-in tool");
    let ctx = ToolContext::new(dir.path().to_path_buf());
    let mut lanes = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let (read, ctx) = (read.clone(), ctx.clone());
        lanes.spawn(async move {
            for _ in 0..25 {
                let output = read.execute(&json!({"path": "note.txt"}), &ctx).await;
                assert!(!output.is_error, "{}", output.content);
            }
        });
    }
    while let Some(lane) = lanes.join_next().await {
        lane.unwrap();
    }
    stop.store(true, Ordering::Relaxed);

    let (reopens, locked) = reopening.join().unwrap();
    assert!(reopens > 0, "the ledger was never reopened");
    assert_eq!(
        locked, 0,
        "{locked} of {reopens} reopens found the ledger locked while workers spawned"
    );
}
