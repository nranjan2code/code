use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use tokio::sync::mpsc;

use crate::error::LlmError;
use crate::types::AssistantMessage;

#[derive(Debug, Clone, serde::Serialize)]
pub enum StreamEvent {
    Start {
        partial: AssistantMessage,
    },
    TextDelta {
        delta: String,
        partial: AssistantMessage,
    },
    ThinkingDelta {
        delta: String,
        partial: AssistantMessage,
    },
    ToolUseStart {
        index: usize,
        id: String,
        name: String,
        partial: AssistantMessage,
    },
    ToolInputDelta {
        index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    End {
        message: AssistantMessage,
    },
}

impl StreamEvent {
    pub fn partial(&self) -> &AssistantMessage {
        match self {
            StreamEvent::Start { partial }
            | StreamEvent::TextDelta { partial, .. }
            | StreamEvent::ThinkingDelta { partial, .. }
            | StreamEvent::ToolUseStart { partial, .. }
            | StreamEvent::ToolInputDelta { partial, .. } => partial,
            StreamEvent::End { message } => message,
        }
    }
}

enum Terminal {
    Message(AssistantMessage),
    Error(LlmError),
}

enum Wire {
    Event(StreamEvent),
    Terminal(Terminal),
}

pub struct EventSink {
    tx: mpsc::Sender<Wire>,
    closed: bool,
}

impl EventSink {
    pub fn push(&self, event: StreamEvent) {
        if self.closed {
            return;
        }
        let _ = self.tx.try_send(Wire::Event(event));
    }

    /// Terminal delivery is awaited, never `try_send`: a full buffer at
    /// message_stop must not turn a completed model turn into
    /// "stream ended without a terminal event". Callers run inside the
    /// provider's spawned stream task, so awaiting here is safe.
    pub async fn close_message(&mut self, message: AssistantMessage) {
        self.closed = true;
        let _ = self
            .tx
            .send(Wire::Terminal(Terminal::Message(message)))
            .await;
    }

    pub async fn close_error(&mut self, error: LlmError) {
        self.closed = true;
        let _ = self.tx.send(Wire::Terminal(Terminal::Error(error))).await;
    }
}

pub struct EventStream {
    rx: mpsc::Receiver<Wire>,
    terminal: Option<Terminal>,
}

pub fn channel(buffer: usize) -> (EventSink, EventStream) {
    let (tx, rx) = mpsc::channel(buffer.max(1));
    (
        EventSink { tx, closed: false },
        EventStream { rx, terminal: None },
    )
}

impl EventStream {
    pub async fn result(mut self) -> Result<AssistantMessage, LlmError> {
        use futures::StreamExt;
        while let Some(_event) = self.next().await {}
        match self.terminal.take() {
            Some(Terminal::Message(m)) => Ok(m),
            Some(Terminal::Error(e)) => Err(e),
            None => Err(LlmError::Parse(
                "stream ended without a terminal event".into(),
            )),
        }
    }
}

impl Stream for EventStream {
    type Item = StreamEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if self.terminal.is_some() {
                return Poll::Ready(None);
            }
            match self.rx.poll_recv(cx) {
                Poll::Ready(Some(Wire::Event(e))) => return Poll::Ready(Some(e)),
                Poll::Ready(Some(Wire::Terminal(t))) => {
                    self.terminal = Some(t);
                    continue;
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
