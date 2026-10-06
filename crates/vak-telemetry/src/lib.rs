//! Content-free structured telemetry (data-architecture plan M5,
//! docs/design/73-data-architecture-and-lifecycle.md §9).
//!
//! Library code reports with `tracing` events and spans; a process installs
//! one subscriber with [`init`]. Every event becomes one JSON line: its time,
//! level, target and message, its own fields, and the fields of the spans
//! around it, with the run's `trace_id` lifted to the top. Telemetry carries
//! no content: a field whose name is not in [`ALLOWED_FIELDS`] is written as
//! [`WITHHELD`], whatever it held, because paths, URLs, file names, titles,
//! prompts and error text are content (doc 79 §6). A message is the literal
//! a call site wrote (a test keeps formatting out of library messages). A
//! span's close is its own line with its duration, for a run's waterfall.
//!
//! Lines go to `<logs>/vak-<service>.jsonl`, rotated by size, and a short
//! human form goes to stderr when it is a terminal.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

/// Field names telemetry may carry: ids, kinds, sizes, digests, durations
/// and outcomes. A name outside this list is withheld.
pub const ALLOWED_FIELDS: &[&str] = &[
    "message",
    "trace_id",
    "run",
    "turn",
    "step",
    "span",
    "session",
    "agent",
    "effect",
    "trigger",
    "slot",
    "owner",
    "stream",
    "process",
    "service",
    "surface",
    "bot",
    "provider",
    "model",
    "tool",
    "call",
    "kind",
    "status",
    "outcome",
    "code",
    "error_kind",
    "attempt",
    "count",
    "bytes",
    "digest",
    "duration_ms",
    "port",
    "cause",
];

/// What a withheld field's value is written as.
pub const WITHHELD: &str = "[withheld]";

/// Whether telemetry may carry a field called `name`.
pub fn allowed(name: &str) -> bool {
    ALLOWED_FIELDS.contains(&name)
}

/// The kind of an error, for an `error_kind` field: the variant or type
/// name that leads its `Debug` form (`Fenced { .. }` reads `Fenced`, an
/// `io::Error` its `ErrorKind`), never its text, which can name paths,
/// URLs or content. An error that is only text reads `unclassified`.
pub fn error_kind(error: &dyn std::fmt::Debug) -> String {
    let debug = format!("{error:?}");
    if let Some(kind) = debug.find("kind: ").map(|at| &debug[at + 6..]).filter(|_| {
        debug.starts_with("Os {") || debug.starts_with("Custom {") || debug.starts_with("Kind(")
    }) {
        let kind: String = kind
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !kind.is_empty() {
            return kind;
        }
    }
    let lead: String = debug
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if lead.is_empty() || !lead.starts_with(|c: char| c.is_ascii_uppercase()) {
        "unclassified".into()
    } else {
        lead
    }
}

#[derive(Default)]
struct Fields(BTreeMap<String, serde_json::Value>);

impl Fields {
    fn put(&mut self, field: &Field, value: serde_json::Value) {
        let name = field.name();
        let value = if allowed(name) {
            value
        } else {
            serde_json::Value::String(WITHHELD.into())
        };
        self.0.insert(name.to_string(), value);
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, serde_json::Value::String(format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, serde_json::Value::String(value.to_string()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, value.into());
    }
}

/// What a span keeps: its fields and when it opened.
struct SpanData {
    fields: Fields,
    opened: Instant,
}

/// Where lines go.
#[derive(Clone)]
pub enum Sink {
    /// A file rotated by size: at most `keep` older files beside it.
    File(Arc<Mutex<RotatingFile>>),
    /// An in-memory buffer, for tests.
    Memory(Arc<Mutex<Vec<u8>>>),
}

impl Sink {
    fn write_line(&self, line: &str) {
        match self {
            Self::File(file) => {
                if let Ok(mut file) = file.lock() {
                    file.write_line(line);
                }
            }
            Self::Memory(buffer) => {
                if let Ok(mut buffer) = buffer.lock() {
                    buffer.extend_from_slice(line.as_bytes());
                    buffer.push(b'\n');
                }
            }
        }
    }
}

/// A JSON-lines file that moves itself to `.1` (and older ones up to
/// `.<keep>`) once it passes `max_bytes`.
pub struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    keep: u32,
    file: Option<File>,
    written: u64,
}

impl RotatingFile {
    pub fn new(path: PathBuf, max_bytes: u64, keep: u32) -> Self {
        let written = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        Self {
            path,
            max_bytes,
            keep,
            file: None,
            written,
        }
    }

    fn rotated(&self, n: u32) -> PathBuf {
        let mut name = self.path.as_os_str().to_os_string();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }

    fn rotate(&mut self) {
        self.file = None;
        for n in (1..self.keep).rev() {
            let _ = fs::rename(self.rotated(n), self.rotated(n + 1));
        }
        if self.keep > 0 {
            let _ = fs::rename(&self.path, self.rotated(1));
        } else {
            let _ = fs::remove_file(&self.path);
        }
        self.written = 0;
    }

    fn write_line(&mut self, line: &str) {
        if self.written > 0 && self.written + line.len() as u64 + 1 > self.max_bytes {
            self.rotate();
        }
        if self.file.is_none() {
            if let Some(parent) = self.path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            self.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .ok();
        }
        if let Some(file) = self.file.as_mut()
            && file.write_all(format!("{line}\n").as_bytes()).is_ok()
        {
            self.written += line.len() as u64 + 1;
        }
    }
}

/// The layer that writes content-free JSON lines.
pub struct ContentFree {
    sink: Sink,
    service: String,
    human: bool,
}

impl ContentFree {
    pub fn new(sink: Sink, service: &str, human: bool) -> Self {
        Self {
            sink,
            service: service.to_string(),
            human,
        }
    }

    /// The fields of the spans around `scope`, outermost first, and the
    /// trace id they carry (a span's `trace_id`, else its `run`).
    fn spans<S>(
        &self,
        ctx: &Context<'_, S>,
        scope: Option<Id>,
    ) -> (Vec<serde_json::Value>, Option<serde_json::Value>)
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let mut spans = Vec::new();
        let mut trace = None;
        if let Some(span) = scope.as_ref().and_then(|id| ctx.span(id)) {
            for span in span.scope().from_root() {
                let extensions = span.extensions();
                let fields = extensions
                    .get::<SpanData>()
                    .map(|data| data.fields.0.clone())
                    .unwrap_or_default();
                if let Some(id) = fields.get("trace_id").or_else(|| fields.get("run")) {
                    trace = Some(id.clone());
                }
                let mut entry = serde_json::Map::new();
                entry.insert("name".into(), span.name().into());
                for (name, value) in fields {
                    entry.insert(name, value);
                }
                spans.push(serde_json::Value::Object(entry));
            }
        }
        (spans, trace)
    }

    fn emit(&self, line: serde_json::Map<String, serde_json::Value>) {
        let text = serde_json::Value::Object(line.clone()).to_string();
        self.sink.write_line(&text);
        if self.human {
            let mut human = String::new();
            for key in ["level", "target", "message", "event"] {
                if let Some(value) = line.get(key).and_then(serde_json::Value::as_str) {
                    let _ = write!(human, "{value} ");
                }
            }
            for (key, value) in &line {
                if !matches!(
                    key.as_str(),
                    "ts" | "level" | "target" | "message" | "event" | "spans" | "service"
                ) {
                    let _ = write!(human, "{key}={value} ");
                }
            }
            let _ = writeln!(std::io::stderr(), "{}", human.trim_end());
        }
    }
}

impl<S> Layer<S> for ContentFree
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanData {
                fields,
                opened: Instant::now(),
            });
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && let Some(data) = span.extensions_mut().get_mut::<SpanData>()
        {
            values.record(&mut data.fields);
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let scope = event
            .parent()
            .cloned()
            .or_else(|| ctx.current_span().id().cloned());
        let (spans, trace) = self.spans(&ctx, scope);
        let mut line = serde_json::Map::new();
        line.insert("ts".into(), chrono::Utc::now().to_rfc3339().into());
        line.insert("level".into(), event.metadata().level().as_str().into());
        line.insert("target".into(), event.metadata().target().into());
        line.insert("service".into(), self.service.clone().into());
        if let Some(trace) = trace {
            line.insert("trace_id".into(), trace);
        }
        for (name, value) in fields.0 {
            line.insert(name, value);
        }
        if !spans.is_empty() {
            line.insert("spans".into(), spans.into());
        }
        self.emit(line);
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else {
            return;
        };
        let (duration_ms, fields) = {
            let extensions = span.extensions();
            let Some(data) = extensions.get::<SpanData>() else {
                return;
            };
            (
                data.opened.elapsed().as_millis() as u64,
                data.fields.0.clone(),
            )
        };
        let parent = span.parent().map(|parent| parent.id());
        let (spans, trace) = self.spans(&ctx, parent);
        let mut line = serde_json::Map::new();
        line.insert("ts".into(), chrono::Utc::now().to_rfc3339().into());
        line.insert("level".into(), "TRACE".into());
        line.insert("target".into(), span.metadata().target().into());
        line.insert("service".into(), self.service.clone().into());
        line.insert("event".into(), "span.close".into());
        line.insert("span".into(), span.name().into());
        line.insert("duration_ms".into(), duration_ms.into());
        let own_trace = fields
            .get("trace_id")
            .or_else(|| fields.get("run"))
            .cloned();
        if let Some(trace) = own_trace.or(trace) {
            line.insert("trace_id".into(), trace);
        }
        for (name, value) in fields {
            line.entry(name).or_insert(value);
        }
        if !spans.is_empty() {
            line.insert("spans".into(), spans.into());
        }
        if self.human {
            // A span's close is for the waterfall, not the terminal.
            self.sink
                .write_line(&serde_json::Value::Object(line).to_string());
        } else {
            self.emit(line);
        }
    }
}

/// The log file of `service`: `<logs>/vak-<service>.jsonl`.
pub fn log_path(service: &str) -> PathBuf {
    vak_config::paths::logs_dir().join(format!("vak-{service}.jsonl"))
}

/// A log file rotates past this many bytes, keeping this many older ones.
pub const ROTATE_BYTES: u64 = 16 * 1024 * 1024;
pub const ROTATE_KEEP: u32 = 5;

/// Installs this process's subscriber for `service`: content-free JSON
/// lines in [`log_path`], and a short human form on stderr when it is a
/// terminal. `VAK_LOG` filters (as `RUST_LOG` would); the default is
/// `info`. A second call does nothing.
pub fn init(service: &str) {
    let filter = tracing_subscriber::EnvFilter::try_new(
        vak_config::get_var("VAK_LOG").unwrap_or_else(|| "info".into()),
    )
    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let sink = Sink::File(Arc::new(Mutex::new(RotatingFile::new(
        log_path(service),
        ROTATE_BYTES,
        ROTATE_KEEP,
    ))));
    let layer = ContentFree::new(sink, service, std::io::stderr().is_terminal());
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init();
}

/// A subscriber writing to `buffer`, for tests that read what was logged.
pub fn capture(buffer: Arc<Mutex<Vec<u8>>>) -> impl Subscriber + Send + Sync {
    tracing_subscriber::registry().with(ContentFree::new(Sink::Memory(buffer), "test", false))
}

/// The lines of a captured buffer, parsed.
pub fn lines(buffer: &Arc<Mutex<Vec<u8>>>) -> Vec<serde_json::Value> {
    let bytes = buffer.lock().map(|bytes| bytes.clone()).unwrap_or_default();
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Reads up to `limit` of the newest lines of `service`'s log (its current
/// file, then its rotated ones), keeping those `keep` accepts, newest first.
pub fn read_lines(
    service: &str,
    limit: usize,
    keep: impl Fn(&serde_json::Value) -> bool,
) -> Vec<serde_json::Value> {
    let path = log_path(service);
    let mut files = vec![path.clone()];
    for n in 1..=ROTATE_KEEP {
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".{n}"));
        files.push(PathBuf::from(name));
    }
    let mut out = Vec::new();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines().rev() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line)
                && keep(&value)
            {
                out.push(value);
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// The services that write a log here.
pub fn services(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| {
                    let name = entry.ok()?.file_name().to_string_lossy().into_owned();
                    name.strip_prefix("vak-")?
                        .strip_suffix(".jsonl")
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}
