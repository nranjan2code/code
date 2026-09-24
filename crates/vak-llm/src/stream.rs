use std::collections::VecDeque;
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

    /// Merges `next` into `self` when they are consecutive deltas of the
    /// same kind (and, for `ToolInputDelta`, the same block `index`):
    /// concatenates `delta` and keeps `next`'s `partial`, since `partial`
    /// is already the full accumulated snapshot and the newer one subsumes
    /// the older. Returns `next` back unmerged for anything else -- a
    /// different kind, a different tool-input index, or a structural event
    /// (`Start`/`ToolUseStart`/`End`) that is never itself a delta. The one
    /// merge rule every lossless forwarding hop uses (`EventSink::push`
    /// here, and `Agent::complete_with_reliability`'s listener forward) to
    /// coalesce a backlog under backpressure instead of dropping events.
    pub fn try_merge(&mut self, next: StreamEvent) -> Option<StreamEvent> {
        match self {
            StreamEvent::TextDelta { delta, partial } => match next {
                StreamEvent::TextDelta {
                    delta: next_delta,
                    partial: next_partial,
                } => {
                    delta.push_str(&next_delta);
                    *partial = next_partial;
                    None
                }
                other => Some(other),
            },
            StreamEvent::ThinkingDelta { delta, partial } => match next {
                StreamEvent::ThinkingDelta {
                    delta: next_delta,
                    partial: next_partial,
                } => {
                    delta.push_str(&next_delta);
                    *partial = next_partial;
                    None
                }
                other => Some(other),
            },
            StreamEvent::ToolInputDelta {
                index,
                delta,
                partial,
            } => match next {
                StreamEvent::ToolInputDelta {
                    index: next_index,
                    delta: next_delta,
                    partial: next_partial,
                } if next_index == *index => {
                    delta.push_str(&next_delta);
                    *partial = next_partial;
                    None
                }
                other => Some(other),
            },
            _ => Some(next),
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
    /// Events `push` could not deliver immediately because the channel was
    /// full, coalesced via `StreamEvent::try_merge` -- never dropped. Never
    /// awaited-send from `push` itself (that would block the provider's own
    /// read loop on a slow listener); flushed opportunistically on the next
    /// `push` and unconditionally, with an awaited send, at close.
    pending: VecDeque<StreamEvent>,
}

impl EventSink {
    /// Best-effort, non-blocking delivery that never drops a still-
    /// deliverable event: a full channel means the listener is behind, not
    /// gone, so `event` is queued locally instead and retried ahead of the
    /// next call. Never awaits -- callers run inside the provider's own
    /// spawned stream-reading task, and a slow listener must not stall that
    /// read loop.
    pub fn push(&mut self, event: StreamEvent) {
        if self.closed {
            return;
        }
        self.drain_pending();
        if !self.pending.is_empty() {
            queue_or_merge(&mut self.pending, event);
            return;
        }
        if let Err(mpsc::error::TrySendError::Full(wire)) = self.tx.try_send(Wire::Event(event))
            && let Wire::Event(event) = wire
        {
            self.pending.push_back(event);
        }
        // `Closed`: nothing could ever be delivered from here on; the event
        // is simply undeliverable, not "dropped under backpressure".
    }

    /// Drains as much of the local backlog as the channel currently
    /// accepts, oldest first, without blocking.
    fn drain_pending(&mut self) {
        while let Some(event) = self.pending.pop_front() {
            match self.tx.try_send(Wire::Event(event)) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(wire)) => {
                    if let Wire::Event(event) = wire {
                        self.pending.push_front(event);
                    }
                    break;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    self.pending.clear();
                    break;
                }
            }
        }
    }

    /// Terminal delivery is awaited, never `try_send`: a full buffer at
    /// message_stop must not turn a completed model turn into
    /// "stream ended without a terminal event". Any backlog `push` queued
    /// under backpressure is flushed first, in order, so the terminal event
    /// is always the last thing the listener sees. Callers run inside the
    /// provider's spawned stream task, so awaiting here is safe.
    pub async fn close_message(&mut self, message: AssistantMessage) {
        self.flush_and_close(Terminal::Message(message)).await;
    }

    pub async fn close_error(&mut self, error: LlmError) {
        self.flush_and_close(Terminal::Error(error)).await;
    }

    async fn flush_and_close(&mut self, terminal: Terminal) {
        self.closed = true;
        for event in self.pending.drain(..) {
            let _ = self.tx.send(Wire::Event(event)).await;
        }
        let _ = self.tx.send(Wire::Terminal(terminal)).await;
    }
}

/// Queues `event`, merging it into the last still-pending event when
/// `StreamEvent::try_merge` allows it -- shared by `push` and `drain_pending`
/// so a slow listener's backlog stays one entry per in-progress block
/// instead of growing one entry per delta.
fn queue_or_merge(pending: &mut VecDeque<StreamEvent>, event: StreamEvent) {
    match pending.back_mut() {
        Some(last) => {
            if let Some(event) = last.try_merge(event) {
                pending.push_back(event);
            }
        }
        None => pending.push_back(event),
    }
}

pub struct EventStream {
    rx: mpsc::Receiver<Wire>,
    terminal: Option<Terminal>,
    guard: Option<Box<dyn Send>>,
}

pub fn channel(buffer: usize) -> (EventSink, EventStream) {
    let (tx, rx) = mpsc::channel(buffer.max(1));
    (
        EventSink {
            tx,
            closed: false,
            pending: VecDeque::new(),
        },
        EventStream {
            rx,
            terminal: None,
            guard: None,
        },
    )
}

impl EventStream {
    pub(crate) fn with_guard<T: Send + 'static>(mut self, guard: T) -> Self {
        self.guard = Some(Box::new(guard));
        self
    }

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::types::{ContentBlock, StopReason, Usage};

    fn msg(text: &str) -> AssistantMessage {
        AssistantMessage {
            content: vec![ContentBlock::text(text.to_string())],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "m".into(),
            response_id: None,
        }
    }

    fn text_delta(delta: &str, text_so_far: &str) -> StreamEvent {
        StreamEvent::TextDelta {
            delta: delta.to_string(),
            partial: msg(text_so_far),
        }
    }

    #[test]
    fn try_merge_concatenates_consecutive_text_deltas_and_keeps_newest_partial() {
        let mut a = text_delta("Hel", "Hel");
        let b = text_delta("lo", "Hello");
        assert!(a.try_merge(b).is_none());
        match a {
            StreamEvent::TextDelta { delta, partial } => {
                assert_eq!(delta, "Hello");
                assert_eq!(partial.text_content(), "Hello");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn try_merge_refuses_different_kinds_and_leaves_self_untouched() {
        let mut a = text_delta("Hel", "Hel");
        let b = StreamEvent::ToolUseStart {
            index: 0,
            id: "1".into(),
            name: "t".into(),
            partial: AssistantMessage::empty("m"),
        };
        let unmerged = a.try_merge(b);
        assert!(matches!(unmerged, Some(StreamEvent::ToolUseStart { .. })));
        match a {
            StreamEvent::TextDelta { delta, .. } => assert_eq!(delta, "Hel"),
            other => panic!("self must be untouched, got {other:?}"),
        }
    }

    #[test]
    fn try_merge_refuses_tool_input_deltas_at_different_indices() {
        let mut a = StreamEvent::ToolInputDelta {
            index: 0,
            delta: "a".into(),
            partial: AssistantMessage::empty("m"),
        };
        let b = StreamEvent::ToolInputDelta {
            index: 1,
            delta: "b".into(),
            partial: AssistantMessage::empty("m"),
        };
        assert!(a.try_merge(b).is_some());
        match a {
            StreamEvent::ToolInputDelta { delta, .. } => assert_eq!(delta, "a"),
            other => panic!("self must be untouched, got {other:?}"),
        }
    }

    #[test]
    fn try_merge_concatenates_tool_input_deltas_at_the_same_index() {
        let mut a = StreamEvent::ToolInputDelta {
            index: 2,
            delta: "{\"a\":".into(),
            partial: AssistantMessage::empty("m"),
        };
        let b = StreamEvent::ToolInputDelta {
            index: 2,
            delta: "1}".into(),
            partial: AssistantMessage::empty("m"),
        };
        assert!(a.try_merge(b).is_none());
        match a {
            StreamEvent::ToolInputDelta { delta, .. } => assert_eq!(delta, "{\"a\":1}"),
            other => panic!("expected ToolInputDelta, got {other:?}"),
        }
    }

    /// The lossless-streaming contract (docs/design/68-context-engine.md
    /// §6): a channel with room for exactly one event, pushed far faster
    /// than a slow consumer drains it, still delivers every character —
    /// concatenating every `TextDelta` the consumer receives (in order)
    /// reconstructs the exact text that was pushed, and the terminal
    /// message carries the same text.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_slow_consumer_over_a_tiny_channel_still_receives_every_delta_losslessly() {
        use futures::StreamExt;

        let (mut sink, mut rx) = channel(1);
        let words: Vec<String> = (0..300).map(|n| format!("w{n} ")).collect();
        let expected: String = words.concat();
        let pieces = words.clone();
        tokio::spawn(async move {
            let mut acc = String::new();
            for word in &pieces {
                acc.push_str(word);
                sink.push(StreamEvent::TextDelta {
                    delta: word.clone(),
                    partial: msg(&acc),
                });
            }
            sink.close_message(msg(&acc)).await;
        });

        let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut received = String::new();
            while let Some(event) = rx.next().await {
                // A deliberately slow consumer: sleep after every receive
                // so the producer races far ahead and backpressure bites.
                tokio::time::sleep(std::time::Duration::from_micros(200)).await;
                if let StreamEvent::TextDelta { delta, .. } = event {
                    received.push_str(&delta);
                }
            }
            received
        })
        .await
        .expect("a lossless slow consumer must still finish promptly");

        assert_eq!(result, expected, "no delta was dropped or reordered");
        let message = rx.result().await.expect("terminal message");
        assert_eq!(message.text_content(), expected);
    }
}
