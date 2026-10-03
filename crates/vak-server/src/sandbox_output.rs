//! An execution's output is persisted as objects, not as one record per
//! chunk (plan M3b slice 2). Chunks of one stream gather until
//! [`CHUNK_BYTES`] or the end of the execution, then become one
//! conversation-scoped object that a record names. Live delivery is
//! unchanged: the client still receives every chunk as it arrives.

use std::collections::BTreeMap;
use std::sync::Arc;
use vak_session::objects::{ObjectRef, Objects, conversation_scope};
use vak_tools::SandboxEvent;

pub(crate) const CHUNK_BYTES: usize = 64 * 1024;

/// One stored piece of an execution's stdout or stderr.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum StoredOutput {
    Output {
        execution_id: String,
        /// `stdout` or `stderr`.
        stream: String,
        object: ObjectRef,
    },
}

/// Gathers one session's execution output into objects.
pub(crate) struct OutputSpool {
    /// `None` when the tenant store could not be opened; output is then
    /// not persisted and each lost chunk is logged.
    objects: Option<Arc<dyn Objects>>,
    scope: String,
    pending: BTreeMap<(String, &'static str), String>,
}

impl OutputSpool {
    pub(crate) fn new(objects: Option<Arc<dyn Objects>>, session_id: &str) -> Self {
        Self {
            objects,
            scope: conversation_scope(session_id),
            pending: BTreeMap::new(),
        }
    }

    /// The records to persist for `event`, in order: nothing while output
    /// gathers, one `Output` record per filled chunk, and at the end of an
    /// execution its remaining output followed by the event itself. A chunk
    /// that cannot be stored is dropped from the record with the reason
    /// logged; the run itself is never stopped by it.
    pub(crate) fn record(&mut self, event: &SandboxEvent) -> Vec<String> {
        let mut lines = Vec::new();
        match event {
            SandboxEvent::Stdout {
                execution_id,
                chunk,
            } => self.gather(execution_id, "stdout", chunk, &mut lines),
            SandboxEvent::Stderr {
                execution_id,
                chunk,
            } => self.gather(execution_id, "stderr", chunk, &mut lines),
            SandboxEvent::ExecutionFinished { execution_id, .. } => {
                for stream in ["stdout", "stderr"] {
                    self.flush(execution_id, stream, &mut lines);
                }
                lines.extend(serde_json::to_string(event).ok());
            }
            other => lines.extend(serde_json::to_string(other).ok()),
        }
        lines
    }

    fn gather(
        &mut self,
        execution_id: &str,
        stream: &'static str,
        chunk: &str,
        lines: &mut Vec<String>,
    ) {
        let pending = self
            .pending
            .entry((execution_id.to_string(), stream))
            .or_default();
        pending.push_str(chunk);
        if pending.len() >= CHUNK_BYTES {
            self.flush(execution_id, stream, lines);
        }
    }

    fn flush(&mut self, execution_id: &str, stream: &'static str, lines: &mut Vec<String>) {
        let Some(text) = self.pending.remove(&(execution_id.to_string(), stream)) else {
            return;
        };
        let stored = match &self.objects {
            Some(objects) => objects.put(text.as_bytes(), &self.scope),
            None => Err(vak_session::SessionError::Objects("no object store".into())),
        };
        match stored {
            Ok(object) => lines.extend(
                serde_json::to_string(&StoredOutput::Output {
                    execution_id: execution_id.to_string(),
                    stream: stream.to_string(),
                    object,
                })
                .ok(),
            ),
            Err(error) => {
                eprintln!("[warn] {execution_id} {stream} output not stored: {error}")
            }
        }
    }
}

/// A spool over memory objects, for tests that record execution events.
#[cfg(test)]
pub(crate) fn test_spool() -> OutputSpool {
    OutputSpool::new(
        Some(Arc::new(vak_session::objects::MemoryObjects::default())),
        "test",
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn out(chunk: &str) -> SandboxEvent {
        SandboxEvent::Stdout {
            execution_id: "e1".into(),
            chunk: chunk.into(),
        }
    }

    #[test]
    fn output_is_one_object_per_chunk_and_reads_back_whole() {
        let objects = Arc::new(vak_session::objects::MemoryObjects::default());
        let mut spool = OutputSpool::new(Some(objects.clone()), "s1");
        assert!(spool.record(&out("hello ")).is_empty());
        let filled = spool.record(&out(&"x".repeat(CHUNK_BYTES)));
        assert_eq!(filled.len(), 1, "a full chunk becomes one record");
        assert!(spool.record(&out("tail")).is_empty());
        let end = spool.record(&SandboxEvent::ExecutionFinished {
            execution_id: "e1".into(),
            exit_code: 0,
            duration_ms: 1,
            artifacts: Vec::new(),
        });
        assert_eq!(end.len(), 2, "the rest of the output, then the event");
        assert!(end[1].contains("ExecutionFinished"));

        let text: String = filled
            .iter()
            .chain(&end[..1])
            .map(|line| {
                let StoredOutput::Output { object, .. } = serde_json::from_str(line).unwrap();
                String::from_utf8(objects.get(&object, &conversation_scope("s1")).unwrap()).unwrap()
            })
            .collect();
        assert_eq!(text, format!("hello {}tail", "x".repeat(CHUNK_BYTES)));
    }
}
