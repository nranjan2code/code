//! Persistent client for the isolated delivery renderer process.

use crate::worker::{WORKER_PROTOCOL_VERSION, WORKER_SUBCOMMAND, WorkerRequest, WorkerResponse};
use crate::{DeliveryJob, DeliveryPacket};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

pub struct WorkerClient {
    executable: PathBuf,
    timeout: Duration,
    process: Mutex<Option<WorkerProcess>>,
}

struct WorkerProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Debug)]
pub enum WorkerClientError {
    Spawn(String),
    Protocol(String),
    Timeout,
    Render(String),
}

impl std::fmt::Display for WorkerClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(message) => write!(formatter, "delivery worker could not start: {message}"),
            Self::Protocol(message) => {
                write!(formatter, "delivery worker protocol failed: {message}")
            }
            Self::Timeout => write!(formatter, "delivery worker timed out"),
            Self::Render(message) => write!(formatter, "delivery rendering failed: {message}"),
        }
    }
}

impl std::error::Error for WorkerClientError {}

impl WorkerClient {
    pub fn new(executable: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            executable: executable.into(),
            timeout,
            process: Mutex::new(None),
        }
    }

    pub fn current_exe(timeout: Duration) -> Result<Self, WorkerClientError> {
        std::env::current_exe()
            .map(|path| Self::new(path, timeout))
            .map_err(|error| WorkerClientError::Spawn(error.to_string()))
    }

    /// Render through a persistent child. A broken or stale protocol stream is
    /// discarded and retried once with a fresh process.
    pub async fn render(&self, job: &DeliveryJob) -> Result<DeliveryPacket, WorkerClientError> {
        let mut process = self.process.lock().await;
        let mut last_error = None;
        for _ in 0..2 {
            if process.is_none() {
                *process = Some(spawn_worker(&self.executable).await?);
            }
            let Some(worker) = process.as_mut() else {
                return Err(WorkerClientError::Spawn(
                    "worker process was not retained".into(),
                ));
            };
            match tokio::time::timeout(self.timeout, exchange(worker, job)).await {
                Ok(Ok(packet)) => return Ok(packet),
                Ok(Err(error)) => last_error = Some(error),
                Err(_) => last_error = Some(WorkerClientError::Timeout),
            }
            if let Some(mut failed) = process.take() {
                let _ = failed.child.kill().await;
            }
        }
        Err(last_error.unwrap_or_else(|| {
            WorkerClientError::Protocol("worker retry ended without a response".into())
        }))
    }
}

async fn spawn_worker(executable: &Path) -> Result<WorkerProcess, WorkerClientError> {
    let mut child = Command::new(executable)
        .arg(WORKER_SUBCOMMAND)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| WorkerClientError::Spawn(error.to_string()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| WorkerClientError::Spawn("worker stdin is unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| WorkerClientError::Spawn("worker stdout is unavailable".into()))?;
    Ok(WorkerProcess {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    })
}

async fn exchange(
    worker: &mut WorkerProcess,
    job: &DeliveryJob,
) -> Result<DeliveryPacket, WorkerClientError> {
    let request = WorkerRequest {
        protocol_version: WORKER_PROTOCOL_VERSION,
        job: job.clone(),
    };
    let mut line = serde_json::to_string(&request)
        .map_err(|error| WorkerClientError::Protocol(error.to_string()))?;
    line.push('\n');
    worker
        .stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|error| WorkerClientError::Protocol(error.to_string()))?;
    worker
        .stdin
        .flush()
        .await
        .map_err(|error| WorkerClientError::Protocol(error.to_string()))?;

    let mut response_line = String::new();
    let bytes = worker
        .stdout
        .read_line(&mut response_line)
        .await
        .map_err(|error| WorkerClientError::Protocol(error.to_string()))?;
    if bytes == 0 {
        return Err(WorkerClientError::Protocol(
            "worker closed its response stream".into(),
        ));
    }
    let response: WorkerResponse = serde_json::from_str(&response_line)
        .map_err(|error| WorkerClientError::Protocol(error.to_string()))?;
    if response.protocol_version != WORKER_PROTOCOL_VERSION {
        return Err(WorkerClientError::Protocol(format!(
            "unsupported response version {}",
            response.protocol_version
        )));
    }
    if response.job_id.as_deref() != Some(job.job_id.as_str()) {
        return Err(WorkerClientError::Protocol(
            "response job id did not match request".into(),
        ));
    }
    match (response.packet, response.error) {
        (Some(packet), None) => Ok(packet),
        (_, Some(error)) => Err(WorkerClientError::Render(error)),
        _ => Err(WorkerClientError::Protocol(
            "response contained neither packet nor error".into(),
        )),
    }
}
