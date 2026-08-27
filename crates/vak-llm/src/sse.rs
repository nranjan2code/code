#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
    cursor: usize,
    pending_event: Option<String>,
    pending_data: Option<String>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    pub fn next_frame(&mut self) -> Option<SseFrame> {
        loop {
            let Some(nl) = self.buf[self.cursor..].iter().position(|&b| b == b'\n') else {
                self.compact();
                return None;
            };
            let line_start = self.cursor;
            let line_end = self.cursor + nl;
            let mut line = &self.buf[line_start..line_end];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            self.cursor = line_end + 1;

            if line.is_empty() {
                let had_content = self.pending_event.is_some() || self.pending_data.is_some();
                if !had_content {
                    continue;
                }
                let frame = SseFrame {
                    event: self.pending_event.take(),
                    data: self.pending_data.take().unwrap_or_default(),
                };
                self.compact();
                return Some(frame);
            }

            // Disjoint field borrows: `line` points into `buf` while the
            // pending fields are mutated.
            Self::ingest_line(line, &mut self.pending_event, &mut self.pending_data);
        }
    }

    /// Emit a frame that is still buffered when the response body ends.
    ///
    /// An SSE frame is terminated by a blank line, but a provider or proxy can
    /// close the body immediately after the final `data:` line. Without this
    /// flush that frame is silently discarded — and it is the last frame of
    /// the answer, so short replies lose almost all of their text and long
    /// ones lose their final token.
    pub fn finish(&mut self) -> Option<SseFrame> {
        // A trailing line with no newline never reached `next_frame`.
        if self.cursor < self.buf.len() {
            let mut line = &self.buf[self.cursor..];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            Self::ingest_line(line, &mut self.pending_event, &mut self.pending_data);
            self.cursor = self.buf.len();
        }
        self.compact();
        if self.pending_event.is_none() && self.pending_data.is_none() {
            return None;
        }
        Some(SseFrame {
            event: self.pending_event.take(),
            data: self.pending_data.take().unwrap_or_default(),
        })
    }

    /// Fold one non-terminating line into the frame being assembled.
    ///
    /// Takes the pending fields rather than `&mut self` so callers can hold a
    /// slice into `buf` while updating them.
    fn ingest_line(
        line: &[u8],
        pending_event: &mut Option<String>,
        pending_data: &mut Option<String>,
    ) {
        if line.is_empty() || line[0] == b':' {
            return;
        }
        let (field, value) = match line.iter().position(|&b| b == b':') {
            Some(colon) => {
                let mut v = &line[colon + 1..];
                if v.first() == Some(&b' ') {
                    v = &v[1..];
                }
                (&line[..colon], v)
            }
            None => (line, &b""[..]),
        };
        match field {
            b"event" => {
                if let Ok(name) = std::str::from_utf8(value) {
                    *pending_event = Some(name.to_string());
                }
            }
            b"data" => {
                if let Ok(text) = std::str::from_utf8(value) {
                    match pending_data {
                        Some(existing) => {
                            existing.push('\n');
                            existing.push_str(text);
                        }
                        None => *pending_data = Some(text.to_string()),
                    }
                }
            }
            _ => {}
        }
    }

    fn compact(&mut self) {
        if self.cursor > 0 {
            self.buf.drain(..self.cursor);
            self.cursor = 0;
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_frames_decode_in_order() {
        let mut decoder = SseDecoder::new();
        decoder.push(b"event: a\ndata: one\n\ndata: two\n\n");
        assert_eq!(
            decoder.next_frame(),
            Some(SseFrame {
                event: Some("a".into()),
                data: "one".into()
            })
        );
        assert_eq!(
            decoder.next_frame(),
            Some(SseFrame {
                event: None,
                data: "two".into()
            })
        );
        assert_eq!(decoder.next_frame(), None);
        assert_eq!(decoder.finish(), None);
    }

    #[test]
    fn a_body_closed_after_the_last_data_line_still_yields_that_frame() {
        // Real providers (opencode-zen among them) end the body right after
        // the final content chunk. Dropping it truncated every answer.
        let mut decoder = SseDecoder::new();
        decoder.push(b"data: one\n\ndata: two\n");
        assert_eq!(
            decoder.next_frame(),
            Some(SseFrame {
                event: None,
                data: "one".into()
            })
        );
        assert_eq!(decoder.next_frame(), None, "frame is not yet terminated");
        assert_eq!(
            decoder.finish(),
            Some(SseFrame {
                event: None,
                data: "two".into()
            }),
            "the final frame must survive end-of-stream"
        );
        assert_eq!(decoder.finish(), None, "finish is not repeatable");
    }

    #[test]
    fn a_body_closed_mid_line_still_yields_that_frame() {
        let mut decoder = SseDecoder::new();
        decoder.push(b"data: tail");
        assert_eq!(decoder.next_frame(), None);
        assert_eq!(
            decoder.finish(),
            Some(SseFrame {
                event: None,
                data: "tail".into()
            })
        );
    }

    #[test]
    fn finish_ignores_comments_and_blank_tails() {
        let mut decoder = SseDecoder::new();
        decoder.push(b"data: one\n\n: keep-alive\n");
        assert_eq!(
            decoder.next_frame(),
            Some(SseFrame {
                event: None,
                data: "one".into()
            })
        );
        assert_eq!(decoder.finish(), None);
    }
}
