// A test's output is for the person running it.
#![allow(clippy::disallowed_macros)]
//! M5 exit tests that need no running server: lines are JSON with their
//! trace fields, telemetry carries no content, and library code reports
//! through `tracing` with literal messages, never `eprintln!`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn captured(work: impl FnOnce()) -> Vec<serde_json::Value> {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    tracing::subscriber::with_default(vak_telemetry::capture(buffer.clone()), work);
    vak_telemetry::lines(&buffer)
}

#[test]
fn log_lines_are_json_with_trace_fields() {
    let lines = captured(|| {
        let run = tracing::info_span!("run", trace_id = "run_a", run = "run_a", agent = "vak");
        let _run = run.enter();
        let turn = tracing::info_span!("turn", turn = "trn_1");
        let _turn = turn.enter();
        tracing::warn!(effect = "eff_1", attempt = 2u64, "delivery failed");
    });
    let event = lines
        .iter()
        .find(|line| line["message"] == "delivery failed")
        .expect("the event is a line");
    for key in ["ts", "level", "target", "service"] {
        assert!(event.get(key).is_some(), "{key} in {event}");
    }
    assert_eq!(event["level"], "WARN");
    assert_eq!(event["trace_id"], "run_a");
    assert_eq!(event["effect"], "eff_1");
    assert_eq!(event["attempt"], 2);
    assert_eq!(event["spans"][0]["name"], "run");
    assert_eq!(event["spans"][1]["turn"], "trn_1");
    // Each span's close is a line with its duration and the same trace.
    let closes: Vec<_> = lines
        .iter()
        .filter(|line| line["event"] == "span.close")
        .collect();
    assert_eq!(closes.len(), 2);
    assert!(closes.iter().all(|line| line["trace_id"] == "run_a"));
    assert!(closes.iter().all(|line| line["duration_ms"].is_u64()));
}

#[test]
fn forwarded_lines_nest_under_the_caller_and_stay_content_free() {
    // What the worker captured: an execution span's close and an event,
    // one with a field a worker should never have written.
    let worker = Arc::new(Mutex::new(Vec::new()));
    tracing::subscriber::with_default(vak_telemetry::capture_as(worker.clone(), "worker"), || {
        let span = tracing::info_span!("execution", trace_id = "run_f", tool = "bash");
        let _span = span.enter();
        tracing::info!(count = 1u64, "a command ran");
    });
    let mut raw = vak_telemetry::raw_lines(&worker, 64, 64 * 1024);
    raw.push(
        r#"{"level":"INFO","message":"x","path":"/Users/someone/CANARY","spans":[{"name":"execution","url":"https://canary.test"}],"bytes":{"nested":"CANARY"}}"#
            .into(),
    );
    let lines = captured(|| {
        let call = tracing::info_span!("tool_call", trace_id = "run_f", tool = "bash");
        let _call = call.enter();
        for line in &raw {
            vak_telemetry::forward(line);
        }
    });
    let text: String = lines.iter().map(|line| line.to_string()).collect();
    assert!(!text.contains("CANARY"), "{text}");
    assert!(!text.contains("canary.test"), "{text}");
    let close = lines
        .iter()
        .find(|line| line["event"] == "span.close" && line["span"] == "execution")
        .expect("the worker's span close is forwarded");
    assert_eq!(close["service"], "worker");
    assert_eq!(close["trace_id"], "run_f");
    assert_eq!(close["spans"][0]["name"], "tool_call");
    let event = lines
        .iter()
        .find(|line| line["message"] == "a command ran")
        .expect("the worker's event is forwarded");
    let names: Vec<_> = event["spans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|span| span["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["tool_call", "execution"]);
    // The forwarding event itself is not a line.
    assert!(lines.iter().all(|line| line["message"] != "forwarded"));
}

#[test]
fn telemetry_carries_no_content() {
    let canaries = [
        "sk-ant-CANARY-SECRET",
        "/Users/someone/Private/CANARY-FILE.docx",
        "https://canary.example.test/secret?token=1",
        "CANARY prompt: tell me about my diagnosis",
    ];
    let lines = captured(|| {
        let span = tracing::info_span!("run", run = "run_b", path = canaries[1]);
        let _span = span.enter();
        tracing::error!(
            error = canaries[0],
            url = canaries[2],
            prompt = canaries[3],
            file = canaries[1],
            title = "CANARY title",
            run = "run_b",
            "tool call failed"
        );
    });
    let text: String = lines.iter().map(|line| line.to_string()).collect();
    for canary in canaries.iter().chain(["CANARY title"].iter()) {
        assert!(!text.contains(canary), "{canary} reached telemetry: {text}");
    }
    let event = lines
        .iter()
        .find(|line| line["message"] == "tool call failed")
        .unwrap();
    assert_eq!(event["error"], vak_telemetry::WITHHELD);
    assert_eq!(event["run"], "run_b");
}

#[test]
fn an_error_kind_names_the_variant_never_the_text() {
    #[derive(Debug)]
    #[allow(dead_code)]
    enum Failure {
        Fenced { held: u64 },
        Io(String),
    }
    assert_eq!(
        vak_telemetry::error_kind(&Failure::Fenced { held: 3 }),
        "Fenced"
    );
    assert_eq!(
        vak_telemetry::error_kind(&Failure::Io("/Users/x/CANARY".into())),
        "Io"
    );
    assert_eq!(
        vak_telemetry::error_kind(&"/Users/x/CANARY".to_string()),
        "unclassified"
    );
    let io = std::io::Error::new(std::io::ErrorKind::NotFound, "/Users/x/CANARY");
    assert_eq!(vak_telemetry::error_kind(&io), "NotFound");
}

#[test]
fn a_log_file_rotates_past_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vak-test.jsonl");
    let file = Arc::new(Mutex::new(vak_telemetry::RotatingFile::new(
        path.clone(),
        200,
        2,
    )));
    let layer = vak_telemetry::ContentFree::new(vak_telemetry::Sink::File(file), "test", false);
    use tracing_subscriber::prelude::*;
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        for count in 0..20u64 {
            tracing::info!(count, "tick");
        }
    });
    assert!(path.exists());
    assert!(dir.path().join("vak-test.jsonl.1").exists());
    assert!(dir.path().join("vak-test.jsonl.2").exists());
    assert!(
        !dir.path().join("vak-test.jsonl.3").exists(),
        "only two are kept"
    );
    assert!(std::fs::metadata(&path).unwrap().len() <= 200);
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Crates and files whose stderr is a person's: the CLI, the terminal and
/// desktop apps, the tray, binaries, and helper processes whose stderr is
/// their parent's to read.
fn user_output(path: &Path) -> bool {
    let text = path.to_string_lossy();
    [
        "crates/vak/",
        "crates/vak-terminal/",
        "crates/vak-desktop/",
        "crates/vak-tray/",
        "/src/bin/",
        "crates/vak-sandbox/src/landlock.rs",
        "/fuzz/",
    ]
    .iter()
    .any(|part| text.contains(part))
}

fn library_sources() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![workspace().join("crates")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if ![
                    "target",
                    "node_modules",
                    "dist",
                    "dist-web",
                    "tests",
                    "benches",
                ]
                .contains(&name.as_str())
                {
                    stack.push(path);
                }
            } else if name.ends_with(".rs")
                && path.to_string_lossy().contains("/src/")
                && !user_output(&path)
            {
                found.push(path);
            }
        }
    }
    found
}

/// The lines of `source` outside `#[cfg(test)]` modules (from the `mod`
/// line after the attribute to the `}` at that line's indentation).
fn shipped_lines(source: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() == "#[cfg(test)]" {
            // Skip the attributes between, including ones over several lines.
            let mut next = index + 1;
            let mut open = 0i64;
            while next < lines.len() && (open > 0 || lines[next].trim_start().starts_with("#[")) {
                open += lines[next].matches('[').count() as i64;
                open -= lines[next].matches(']').count() as i64;
                next += 1;
            }
            let head = lines.get(next).map(|line| line.trim_start()).unwrap_or("");
            if (head.starts_with("mod ")
                || head.starts_with("pub mod ")
                || head.starts_with("pub(crate) mod "))
                && head.ends_with('{')
            {
                // Formatted source closes a module at its own indentation.
                let indent = &lines[next][..lines[next].len() - lines[next].trim_start().len()];
                let close = format!("{indent}}}");
                let end = lines[next..]
                    .iter()
                    .position(|line| *line == close)
                    .map_or(lines.len(), |offset| next + offset);
                index = end + 1;
                continue;
            }
        }
        out.push((index + 1, lines[index]));
        index += 1;
    }
    out
}

#[test]
fn library_crates_have_no_eprintln() {
    let mut found = Vec::new();
    for path in library_sources() {
        let source = std::fs::read_to_string(&path).unwrap();
        for (number, line) in shipped_lines(&source) {
            let code = line.split("//").next().unwrap_or("");
            let printed =
                |name: &str| code.contains(name) && !code.contains(&format!("{name}\\\""));
            if printed("eprintln!(") || printed("println!(") {
                found.push(format!("{}:{number}", path.display()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "library code reports through tracing: {found:#?}"
    );
}

/// The string literals of the macro call whose arguments start `call`, up
/// to its closing parenthesis (strings skipped while matching).
fn call_literals(call: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut depth = 1i32;
    let mut chars = call.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut literal = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            literal.push(c);
                            if let Some(next) = chars.next() {
                                literal.push(next);
                            }
                        }
                        '"' => break,
                        _ => literal.push(c),
                    }
                }
                literals.push(literal);
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    literals
}

/// A library message is the literal its call site wrote: anything that
/// varies goes in an allowlisted field, never formatted into the text.
#[test]
fn library_log_messages_are_literals() {
    let mut found = Vec::new();
    for path in library_sources() {
        let source = std::fs::read_to_string(&path).unwrap();
        let shipped: String = shipped_lines(&source)
            .iter()
            .map(|(_, line)| format!("{line}\n"))
            .collect();
        for macro_name in [
            "tracing::trace!(",
            "tracing::debug!(",
            "tracing::info!(",
            "tracing::warn!(",
            "tracing::error!(",
        ] {
            let mut rest = shipped.as_str();
            while let Some(at) = rest.find(macro_name) {
                rest = &rest[at + macro_name.len()..];
                let literals = call_literals(rest);
                if literals
                    .last()
                    .is_some_and(|message| message.replace("{{", "").contains('{'))
                {
                    found.push(format!("{}: {:?}", path.display(), literals.last()));
                }
            }
        }
    }
    assert!(found.is_empty(), "messages must be literals: {found:#?}");
}
