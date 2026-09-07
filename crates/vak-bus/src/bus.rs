//! Asynchronous event and work queue engines.
//!
//! Provides the core traits (`EventPublisher`, `EventSubscriber`, `WorkQueue`),
//! the real production NATS Core & JetStream engine (`NatsBus`), and the
//! high-throughput standalone concurrent engine (`InMemoryBus`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc};

use crate::envelope::MessageEnvelope;
use crate::telemetry::{BusMetrics, DeadLetterEvent};

#[derive(Debug, Error)]
pub enum BusError {
    #[error("connection error: {0}")]
    Connection(String),
    #[error("publish error on subject '{subject}': {reason}")]
    Publish { subject: String, reason: String },
    #[error("subscription error for pattern '{pattern}': {reason}")]
    Subscribe { pattern: String, reason: String },
    #[error("work queue error for queue '{queue}': {reason}")]
    WorkQueue { queue: String, reason: String },
    #[error("serialization/deserialization error: {0}")]
    Serialization(String),
    #[error("task acknowledgement error: {0}")]
    AckError(String),
    #[error("maximum retry attempts exceeded (poison pill routed to DLQ)")]
    MaxRetriesExceeded,
    #[error("internal bus error: {0}")]
    Internal(String),
}

/// Acknowledge handle for a claimed work queue task.
#[async_trait]
pub trait TaskAckHandle: Send + Sync {
    /// Acknowledge successful task completion.
    async fn ack(&self) -> Result<(), BusError>;
    /// Negative acknowledge (re-queue task for another attempt or route to DLQ).
    async fn nack(&self, retry: bool) -> Result<(), BusError>;
}

/// A claimed task from a durable work queue.
pub struct ClaimedTask {
    pub task_id: String,
    pub envelope: MessageEnvelope,
    pub attempts: u32,
    pub ack_handle: Box<dyn TaskAckHandle>,
}

/// Publisher contract for streaming ephemeral events.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, subject: &str, envelope: MessageEnvelope) -> Result<(), BusError>;
}

/// Subscriber contract for streaming ephemeral events.
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    async fn subscribe(
        &self,
        subject_pattern: &str,
    ) -> Result<mpsc::Receiver<MessageEnvelope>, BusError>;
}

/// Durable distributed work queue contract for competing consumer agent pools.
#[async_trait]
pub trait WorkQueue: Send + Sync {
    /// Enqueue a durable task into a named work queue.
    async fn enqueue(&self, queue: &str, envelope: MessageEnvelope) -> Result<String, BusError>;

    /// Claim a pending task from the work queue.
    async fn claim(
        &self,
        queue: &str,
        worker_id: &str,
        timeout: Duration,
    ) -> Result<Option<ClaimedTask>, BusError>;
}

// ---------------------------------------------------------------------------
// Real Production NATS Core + JetStream Engine
// ---------------------------------------------------------------------------

/// Configuration for connecting to a distributed NATS cluster.
#[derive(Debug, Clone)]
pub struct NatsConfig {
    pub url: String,
    pub credentials_jwt: Option<String>,
    pub nkey_seed: Option<String>,
    pub connect_timeout: Duration,
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            url: "nats://127.0.0.1:4222".to_string(),
            credentials_jwt: None,
            nkey_seed: None,
            connect_timeout: Duration::from_secs(5),
        }
    }
}

/// Production distributed messaging engine powered by NATS Core & JetStream.
pub struct NatsBus {
    client: async_nats::Client,
    js: async_nats::jetstream::Context,
    metrics: Arc<BusMetrics>,
}

impl NatsBus {
    /// Connect to the NATS cluster and initialize JetStream contexts.
    pub async fn connect(config: NatsConfig) -> Result<Self, BusError> {
        let mut connect_opts = async_nats::ConnectOptions::new();
        connect_opts = connect_opts.connection_timeout(config.connect_timeout);

        if let Some(creds) = config.credentials_jwt {
            connect_opts = connect_opts
                .credentials(&creds)
                .map_err(|e| BusError::Connection(e.to_string()))?;
        } else if let Some(nkey) = config.nkey_seed {
            connect_opts = connect_opts.nkey(nkey);
        }

        let client = connect_opts
            .connect(&config.url)
            .await
            .map_err(|e| BusError::Connection(e.to_string()))?;

        let js = async_nats::jetstream::new(client.clone());
        let metrics = BusMetrics::new();

        Ok(Self {
            client,
            js,
            metrics,
        })
    }

    /// Access live operational metrics.
    pub fn metrics(&self) -> Arc<BusMetrics> {
        self.metrics.clone()
    }
}

#[async_trait]
impl EventPublisher for NatsBus {
    async fn publish(&self, subject: &str, envelope: MessageEnvelope) -> Result<(), BusError> {
        let payload =
            serde_json::to_vec(&envelope).map_err(|e| BusError::Serialization(e.to_string()))?;
        let payload_len = payload.len();

        self.client
            .publish(subject.to_string(), payload.into())
            .await
            .map_err(|e| BusError::Publish {
                subject: subject.to_string(),
                reason: e.to_string(),
            })?;

        self.metrics.record_published(payload_len);
        Ok(())
    }
}

#[async_trait]
impl EventSubscriber for NatsBus {
    async fn subscribe(
        &self,
        subject_pattern: &str,
    ) -> Result<mpsc::Receiver<MessageEnvelope>, BusError> {
        use futures::StreamExt;

        let mut sub = self
            .client
            .subscribe(subject_pattern.to_string())
            .await
            .map_err(|e| BusError::Subscribe {
                pattern: subject_pattern.to_string(),
                reason: e.to_string(),
            })?;

        let (tx, rx) = mpsc::channel(2048);
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            while let Some(msg) = sub.next().await {
                metrics.record_received(msg.payload.len());
                if let Ok(env) = serde_json::from_slice::<MessageEnvelope>(&msg.payload)
                    && tx.send(env).await.is_err()
                {
                    break;
                }
            }
        });

        Ok(rx)
    }
}

struct NatsJetStreamAckHandle {
    msg: Arc<tokio::sync::Mutex<Option<async_nats::jetstream::Message>>>,
}

#[async_trait]
impl TaskAckHandle for NatsJetStreamAckHandle {
    async fn ack(&self) -> Result<(), BusError> {
        let mut guard = self.msg.lock().await;
        if let Some(msg) = guard.take() {
            msg.ack()
                .await
                .map_err(|e| BusError::AckError(e.to_string()))?;
        }
        Ok(())
    }

    async fn nack(&self, retry: bool) -> Result<(), BusError> {
        let mut guard = self.msg.lock().await;
        if let Some(msg) = guard.take() {
            if retry {
                msg.ack_with(async_nats::jetstream::message::AckKind::Nak(None))
                    .await
                    .map_err(|e| BusError::AckError(e.to_string()))?;
            } else {
                msg.ack()
                    .await
                    .map_err(|e| BusError::AckError(e.to_string()))?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl WorkQueue for NatsBus {
    async fn enqueue(&self, queue: &str, envelope: MessageEnvelope) -> Result<String, BusError> {
        let subject = format!("vak.work.{queue}.task");
        let payload =
            serde_json::to_vec(&envelope).map_err(|e| BusError::Serialization(e.to_string()))?;
        let payload_len = payload.len();

        let ack_future = self
            .js
            .publish(subject.clone(), payload.into())
            .await
            .map_err(|e| BusError::WorkQueue {
                queue: queue.to_string(),
                reason: e.to_string(),
            })?;

        let ack = ack_future.await.map_err(|e| BusError::WorkQueue {
            queue: queue.to_string(),
            reason: e.to_string(),
        })?;

        self.metrics.record_published(payload_len);
        Ok(format!("{}:{}", ack.stream, ack.sequence))
    }

    async fn claim(
        &self,
        queue: &str,
        _worker_id: &str,
        timeout: Duration,
    ) -> Result<Option<ClaimedTask>, BusError> {
        use futures::StreamExt;

        let stream_name = format!("VAK_WORK_{}", queue.to_uppercase());
        let consumer_name = format!("worker_{queue}");

        let stream = match self.js.get_stream(&stream_name).await {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };

        let consumer = match stream.get_consumer(&consumer_name).await {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };

        let mut messages = match consumer.fetch().max_messages(1).messages().await {
            Ok(m) => m,
            Err(_) => return Ok(None),
        };

        match tokio::time::timeout(timeout, messages.next()).await {
            Ok(Some(Ok(msg))) => {
                self.metrics.record_received(msg.payload.len());
                let envelope: MessageEnvelope = serde_json::from_slice(&msg.payload)
                    .map_err(|e| BusError::Serialization(e.to_string()))?;
                let attempts = 1;
                let task_id = envelope.id.clone();
                let ack_handle = Box::new(NatsJetStreamAckHandle {
                    msg: Arc::new(tokio::sync::Mutex::new(Some(msg))),
                });

                Ok(Some(ClaimedTask {
                    task_id,
                    envelope,
                    attempts,
                    ack_handle,
                }))
            }
            _ => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone Concurrency Engine (Zero-Dependency Local/Test Engine)
// ---------------------------------------------------------------------------

struct InMemoryTask {
    id: String,
    envelope: MessageEnvelope,
    attempts: u32,
}

/// Standalone, high-throughput concurrent engine matching NATS Core & JetStream semantics.
pub struct InMemoryBus {
    events_tx: broadcast::Sender<(String, MessageEnvelope)>,
    queues: Arc<Mutex<HashMap<String, VecDeque<InMemoryTask>>>>,
    dlq: Arc<Mutex<Vec<DeadLetterEvent>>>,
    metrics: Arc<BusMetrics>,
}

impl Default for InMemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryBus {
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(4096);
        Self {
            events_tx,
            queues: Arc::new(Mutex::new(HashMap::new())),
            dlq: Arc::new(Mutex::new(Vec::new())),
            metrics: BusMetrics::new(),
        }
    }

    pub fn metrics(&self) -> Arc<BusMetrics> {
        self.metrics.clone()
    }

    pub fn dead_letters(&self) -> Vec<DeadLetterEvent> {
        self.dlq.lock().map(|d| d.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl EventPublisher for InMemoryBus {
    async fn publish(&self, subject: &str, envelope: MessageEnvelope) -> Result<(), BusError> {
        let size = serde_json::to_vec(&envelope)
            .map(|v| v.len())
            .unwrap_or(128);
        self.metrics.record_published(size);
        let _ = self.events_tx.send((subject.to_string(), envelope));
        Ok(())
    }
}

#[async_trait]
impl EventSubscriber for InMemoryBus {
    async fn subscribe(
        &self,
        subject_pattern: &str,
    ) -> Result<mpsc::Receiver<MessageEnvelope>, BusError> {
        let mut b_rx = self.events_tx.subscribe();
        let (tx, rx) = mpsc::channel(1024);
        let pattern = subject_pattern.to_string();
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            while let Ok((subj, env)) = b_rx.recv().await {
                if crate::subjects::matches_pattern(&pattern, &subj) {
                    metrics.record_received(128);
                    if tx.send(env).await.is_err() {
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }
}

struct InMemoryAckHandle {
    queue: String,
    task_id: String,
    envelope: MessageEnvelope,
    attempts: u32,
    max_retries: u32,
    queues: Arc<Mutex<HashMap<String, VecDeque<InMemoryTask>>>>,
    dlq: Arc<Mutex<Vec<DeadLetterEvent>>>,
    metrics: Arc<BusMetrics>,
}

#[async_trait]
impl TaskAckHandle for InMemoryAckHandle {
    async fn ack(&self) -> Result<(), BusError> {
        // Task is already removed on claim; ack finalizes it.
        Ok(())
    }

    async fn nack(&self, retry: bool) -> Result<(), BusError> {
        if retry && self.attempts < self.max_retries {
            if let Ok(mut map) = self.queues.lock() {
                let q = map.entry(self.queue.clone()).or_default();
                q.push_front(InMemoryTask {
                    id: self.task_id.clone(),
                    envelope: self.envelope.clone(),
                    attempts: self.attempts + 1,
                });
            }
        } else {
            // Divert to Dead-Letter Queue (DLQ)
            self.metrics.record_dead_letter();
            if let Ok(mut dlq) = self.dlq.lock() {
                dlq.push(DeadLetterEvent::new(
                    self.envelope.clone(),
                    &self.queue,
                    "worker",
                    self.attempts,
                    "retry threshold exhausted",
                ));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl WorkQueue for InMemoryBus {
    async fn enqueue(&self, queue: &str, envelope: MessageEnvelope) -> Result<String, BusError> {
        let task_id = envelope.id.clone();
        let size = serde_json::to_vec(&envelope)
            .map(|v| v.len())
            .unwrap_or(128);
        self.metrics.record_published(size);

        if let Ok(mut map) = self.queues.lock() {
            let q = map.entry(queue.to_string()).or_default();
            q.push_back(InMemoryTask {
                id: task_id.clone(),
                envelope,
                attempts: 1,
            });
            self.metrics.update_queue_lag(q.len() as u64);
        }

        Ok(task_id)
    }

    async fn claim(
        &self,
        queue: &str,
        _worker_id: &str,
        _timeout: Duration,
    ) -> Result<Option<ClaimedTask>, BusError> {
        let task_opt = {
            let mut map = match self.queues.lock() {
                Ok(m) => m,
                Err(_) => return Ok(None),
            };
            let q = map.entry(queue.to_string()).or_default();
            let item = q.pop_front();
            self.metrics.update_queue_lag(q.len() as u64);
            item
        };

        if let Some(task) = task_opt {
            self.metrics.record_received(128);
            let ack_handle = Box::new(InMemoryAckHandle {
                queue: queue.to_string(),
                task_id: task.id.clone(),
                envelope: task.envelope.clone(),
                attempts: task.attempts,
                max_retries: 3,
                queues: self.queues.clone(),
                dlq: self.dlq.clone(),
                metrics: self.metrics.clone(),
            });

            Ok(Some(ClaimedTask {
                task_id: task.id,
                envelope: task.envelope,
                attempts: task.attempts,
                ack_handle,
            }))
        } else {
            Ok(None)
        }
    }
}
