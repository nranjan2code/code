//! A session's sandbox records (candidates, reviews, promotions, undo,
//! workspace checks and revisions) as a record chain in its Agent home
//! (`AgentScope::sandbox_records`). `vak-sandbox` defines the records and
//! knows nothing of where they are kept; this is the one writer and reader,
//! so no record is ever appended to a file directly. A session's execution
//! stream (`AgentScope::sandbox_executions`) is a chain of the same kind.

use std::path::Path;

/// Append `record`, durably, to the chain at `path`.
pub(crate) fn append(
    path: &Path,
    record: &vak_sandbox::DurableRecord,
) -> Result<(), vak_sandbox::Error> {
    let failed = |error: String| vak_sandbox::Error::Io(std::io::Error::other(error));
    let mut row = serde_json::to_value(record).map_err(|error| failed(error.to_string()))?;
    // A record about a session belongs to its conversation (plan M7a-b):
    // the chain keeps its kind, and the record is an object of that scope.
    let session = row
        .pointer("/record/session_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    if let Some(session) = session {
        vak_session::content::seal_fields(&mut row, &session, &["record"])
            .map_err(|error| failed(error.to_string()))?;
    }
    vak_session::chain::RecordChain::at(path)
        .append(&row)
        .map_err(|error| failed(error.to_string()))
}

/// Every record in the chain at `path`, in order; none when it is absent.
/// A row that is not a record is an error, never skipped: a status derived
/// from records with one missing would be a confident wrong answer.
pub(crate) fn load(path: &Path) -> Result<Vec<vak_sandbox::DurableRecord>, vak_sandbox::Error> {
    vak_session::chain::RecordChain::at(path)
        .read::<serde_json::Value>()
        .into_iter()
        .filter_map(|mut row| {
            let invalid = |error: String| {
                vak_sandbox::Error::InvalidPlan(format!("record parse failed: {error}"))
            };
            match vak_session::content::restore(&mut row) {
                Ok(vak_session::content::Restored::Whole) => {}
                // A record of an erased conversation is gone with it.
                Ok(vak_session::content::Restored::Erased) => return None,
                Err(error) => return Some(Err(invalid(error.to_string()))),
            }
            Some(serde_json::from_value(row).map_err(|error| invalid(error.to_string())))
        })
        .collect()
}

/// Append a batch of execution-stream events (JSON rows from the output
/// spool) to the chain at `path`, synced once for the batch.
pub(crate) fn append_events(path: &Path, session_id: &str, lines: &[String]) -> Result<(), String> {
    let rows: Vec<serde_json::Value> = lines
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    vak_session::chain::RecordChain::at(path)
        .of_conversation(session_id)
        .append_all(&rows)
        .map_err(|error| error.to_string())
}

/// The execution-stream events at `path` that decode as `T`, in order.
pub(crate) fn events<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    vak_session::chain::RecordChain::at(path).read()
}
