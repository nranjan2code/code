//! Black-box contracts for the greenfield runtime boundary.
//!
//! These tests intentionally exercise only public APIs.  They are the first
//! line of architecture enforcement: adapters may change, but the runtime
//! must keep ownership, cancellation, and storage semantics stable.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::time::Duration;

use tokio::time::timeout;
use vak_domain::{CapabilityEpoch, ProjectId, RunId, RunStatus};
use vak_runtime::{LeaseKind, RunError, RunSupervisor, Runtime, WorkspaceLeaseManager};
use vak_storage::{BlobStore, StateStore, StorageError};

struct TestDir(std::path::PathBuf);

impl TestDir {
    fn new(label: &str) -> Result<Self, std::io::Error> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("vakcoder-{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn terminal_transition_is_idempotent_and_observable_once() {
    let supervisor = RunSupervisor::new();
    let run_id = RunId::new();
    let handle = supervisor
        .start(run_id.clone(), ProjectId::new())
        .await
        .expect("the first admission succeeds");

    assert!(handle.complete().await.expect("completion is accepted"));
    assert!(
        !handle
            .complete()
            .await
            .expect("duplicate completion is harmless")
    );
    assert!(
        !handle
            .cancelled()
            .await
            .expect("late cancellation is harmless")
    );

    let snapshot = supervisor
        .wait_terminal(&run_id)
        .await
        .expect("terminal state remains queryable");
    assert_eq!(snapshot.status, RunStatus::Completed);
    assert!(snapshot.status.is_terminal());
}

#[tokio::test]
async fn revocation_cancels_old_runs_and_never_poison_new_runs() {
    let supervisor = RunSupervisor::new();
    let old_id = RunId::new();
    let old = supervisor
        .start(old_id.clone(), ProjectId::new())
        .await
        .expect("old run admission succeeds");
    let old_epoch = old.capability_epoch();

    let new_epoch = supervisor.revoke_capabilities().await;
    assert_eq!(new_epoch, CapabilityEpoch(old_epoch.0 + 1));
    assert!(old.is_cancelled());
    assert!(old.check_capability().is_err());
    assert_eq!(
        supervisor
            .snapshot(&old_id)
            .await
            .expect("old run exists")
            .status,
        RunStatus::Cancelling
    );

    let new = supervisor
        .start(RunId::new(), ProjectId::new())
        .await
        .expect("new run admission succeeds");
    assert_eq!(new.capability_epoch(), new_epoch);
    assert!(!new.is_cancelled());
    assert!(new.check_capability().is_ok());
}

#[tokio::test]
async fn cancellation_is_scoped_to_an_admitted_run() {
    let supervisor = RunSupervisor::new();
    let unknown = RunId::new();
    assert_eq!(
        supervisor.cancel(&unknown).await,
        Err(RunError::NotFound(unknown))
    );

    let run = supervisor
        .start(RunId::new(), ProjectId::new())
        .await
        .expect("run admission succeeds");
    assert!(
        supervisor
            .cancel(run.run_id())
            .await
            .expect("cancel succeeds")
    );
    assert!(
        !supervisor
            .cancel(run.run_id())
            .await
            .expect("cancel is idempotent")
    );
    assert!(run.is_cancelled());
}

#[tokio::test]
async fn workspace_leases_conflict_only_within_the_same_project() {
    let manager = WorkspaceLeaseManager::new();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();
    let read_a = manager.acquire(project_a.clone(), LeaseKind::Read).await;

    let write_b = timeout(
        Duration::from_millis(50),
        manager.acquire(project_b, LeaseKind::Write),
    )
    .await
    .expect("different projects do not conflict");
    drop(write_b);

    assert!(
        timeout(
            Duration::from_millis(20),
            manager.acquire(project_a.clone(), LeaseKind::Write),
        )
        .await
        .is_err(),
        "a write waits behind a read in the same project"
    );
    drop(read_a);
    timeout(
        Duration::from_millis(100),
        manager.acquire(project_a, LeaseKind::Write),
    )
    .await
    .expect("the write proceeds after the read releases");
}

#[test]
fn state_store_refuses_unknown_schema_versions() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TestDir::new("schema")?;
    let path = dir.path().join("state.db");
    {
        let store = StateStore::open(&path)?;
        store
            .connection()
            .execute_batch("PRAGMA user_version = 99;")?;
    }

    assert!(matches!(
        StateStore::open(&path),
        Err(StorageError::SchemaVersion(99))
    ));
    Ok(())
}

#[test]
fn blob_store_is_content_addressed_and_does_not_overwrite_existing_content()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TestDir::new("blobs")?;
    let blobs = BlobStore::open(dir.path().join("blobs"))?;
    let first = blobs.put(b"first")?;
    assert_eq!(blobs.put(b"first")?, first);

    let second = blobs.put(b"second")?;
    assert_ne!(first, second);
    assert_eq!(blobs.get(&first)?.as_deref(), Some(b"first".as_slice()));
    assert_eq!(blobs.get(&second)?.as_deref(), Some(b"second".as_slice()));

    // The path is derived solely from the digest returned by `put`; a second
    // write of different bytes cannot replace the first object.
    let first_hex = first.strip_prefix("sha256:").expect("sha256 digest");
    let first_path = dir
        .path()
        .join("blobs")
        .join(&first_hex[..2])
        .join(first_hex);
    assert!(first_path.is_file());
    assert_eq!(fs::read(first_path)?, b"first");
    Ok(())
}

#[tokio::test]
async fn runtime_owns_state_and_blob_roots_under_one_data_home()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TestDir::new("runtime")?;
    let runtime = Runtime::open(dir.path())?;
    assert_eq!(runtime.data_home(), dir.path());
    assert!(dir.path().join("state.db").is_file());
    assert!(dir.path().join("blobs").is_dir());
    Ok(())
}
