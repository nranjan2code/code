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

            if line[0] == b':' {
                continue;
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
                        self.pending_event = Some(name.to_string());
                    }
                }
                b"data" => {
                    if let Ok(text) = std::str::from_utf8(value) {
                        match &mut self.pending_data {
                            Some(existing) => {
                                existing.push('\n');
                                existing.push_str(text);
                            }
                            None => self.pending_data = Some(text.to_string()),
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn compact(&mut self) {
        if self.cursor > 0 {
            self.buf.drain(..self.cursor);
            self.cursor = 0;
        }
    }
}
