//! Goal mode + audited completion (docs/design/42-managed-work-contracts.md).
//!
//! A goal is a durable objective with acceptance criteria. Completion is
//! never self-reported: when the model claims done, the loop audits the
//! claim — deterministic `verify:` criteria run as brokered shell
//! commands, remaining criteria go to one skeptical judge call — and only
//! an audited pass ends the run. Rejections return findings to the
//! executor; the audit budget is capped so this can never trap a run.
//!
//! Regression obligations: bash commands proven green during the run are
//! re-run before any completion claim; a regression rejects the claim.

use vak_llm::{ChatRequest, ContentBlock};

/// One acceptance criterion. `verify:`-prefixed criteria are executed
/// deterministically; everything else goes to the judge.
pub fn is_shell_criterion(criterion: &str) -> bool {
    criterion.trim_start().starts_with("verify:")
}

pub fn shell_command(criterion: &str) -> &str {
    criterion
        .trim_start()
        .strip_prefix("verify:")
        .unwrap_or(criterion)
        .trim()
}

#[derive(Debug, Clone)]
pub struct GoalState {
    pub objective: String,
    pub criteria: Vec<String>,
    /// Remaining audit blocks before the goal degrades to Unverified.
    pub audits_left: u32,
}

pub const AUDIT_SYSTEM: &str = "\
You are a completion auditor for an agent session. You receive the \
session's objective, its acceptance criteria, and a transcript digest of \
what the agent actually did. Judge each criterion independently against \
EVIDENCE IN THE TRANSCRIPT ONLY — never give benefit of the doubt. Reply \
with STRICT JSON and nothing else: \
{\"results\":[{\"criterion\":\"<verbatim criterion>\",\"verdict\":\"pass|fail|unknown\",\"evidence\":\"<short quote or reason>\"}]}";

pub fn audit_prompt(
    objective: &str,
    criteria: &[String],
    transcript_digest: &str,
    workspace_delta: Option<&str>,
) -> String {
    let delta_section = match workspace_delta {
        Some(d) if !d.trim().is_empty() => format!(
            "\nWorkspace delta since run start:\n<workspace_delta>\n{d}\n</workspace_delta>\n"
        ),
        _ => "\n(workspace delta unavailable — judge from transcript evidence only)\n".to_string(),
    };
    format!(
        "Objective: {objective}\n\nAcceptance criteria:\n{}\n\nTranscript digest:\n<transcript>\n{transcript_digest}\n</transcript>\n{delta_section}\n\
         Return the JSON verdict now.",
        criteria
            .iter()
            .map(|c| format!("- {c}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct CriterionVerdict {
    pub criterion: String,
    pub verdict: String,
    pub evidence: String,
}

/// Every balanced top-level `{…}` in `text`, string-aware, so a reply that
/// wraps its JSON in prose or fences, or emits one object per criterion,
/// yields each object on its own.
fn balanced_objects(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            match b {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0
                    && let Some(s) = start.take()
                {
                    out.push(&text[s..=i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// Lenient JSON parse: models wrap verdicts in prose/fences, and — measured
/// live on `gemma4:e2b-mlx` — sometimes emit one `{"results":[…]}` object
/// per criterion back to back. Every balanced object that carries a
/// `results` array contributes; nothing usable => Err (fail-closed).
pub fn parse_verdicts(text: &str) -> Result<Vec<CriterionVerdict>, String> {
    let objects = balanced_objects(text);
    if objects.is_empty() {
        return Err("no JSON object in judge reply".into());
    }
    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut last_error: Option<String> = None;
    for object in objects {
        match serde_json::from_str::<serde_json::Value>(object) {
            Ok(value) => {
                if let Some(array) = value.get("results").and_then(|r| r.as_array()) {
                    results.extend(array.iter().cloned());
                }
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if results.is_empty() {
        return Err(last_error.unwrap_or_else(|| "missing results array".into()));
    }
    let mut out = Vec::new();
    for r in &results {
        out.push(CriterionVerdict {
            criterion: r
                .get("criterion")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            verdict: r
                .get("verdict")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            evidence: r
                .get("evidence")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }
    if out.is_empty() {
        return Err("judge returned no results".into());
    }
    Ok(out)
}

/// Builds the judge request over the digest.
pub fn audit_request(model: &str, prompt: String) -> ChatRequest {
    let mut req = ChatRequest::new(model);
    req.system = Some(AUDIT_SYSTEM.to_string());
    req.messages = vec![vak_llm::Message::user_text(prompt)];
    req.max_tokens = 1024;
    // The provider's default for thinking, deliberately. Measured live on
    // `gemma4:e2b-mlx` with the tolerant parser below: thinking on, 6/6
    // verdicts usable at ~8s; thinking off, 3/6 at ~2s, the rest lost to
    // mis-escaped quotes in the evidence field. A judge that fails closed
    // is worth the latency; the other side-dispatches (classify, compact,
    // handoff, plan) showed no such difference and run without thinking.
    req
}

/// Extracts a bounded digest of the conversation for judging.
pub fn transcript_digest(messages: &[vak_llm::Message], max_chars: usize) -> String {
    // Render tail-first so recent, most-relevant turns survive the cap.
    let mut chunks: Vec<String> = Vec::new();
    let mut used = 0usize;
    for m in messages.iter().rev() {
        let text = render_one(m);
        if used + text.len() > max_chars {
            break;
        }
        used += text.len();
        chunks.push(text);
    }
    chunks.reverse();
    chunks.join("\n")
}

fn render_one(m: &vak_llm::Message) -> String {
    let role = match m.role {
        vak_llm::Role::User => "user",
        vak_llm::Role::Assistant => "assistant",
    };
    let mut out = format!("[{role}] ");
    for b in &m.content {
        match b {
            ContentBlock::Text { text } => out.push_str(text),
            ContentBlock::ToolUse { name, input, .. } => {
                out.push_str(&format!(
                    "[tool-call] {name} {}",
                    serde_json::to_string(input).unwrap_or_default()
                ));
            }
            ContentBlock::ToolResult { content, .. } => {
                let preview: String = content.chars().take(300).collect();
                out.push_str(&format!("[tool-result] {preview}"));
            }
            _ => {}
        }
        out.push('\n');
    }
    out
}

pub const HANDOFF_SYSTEM: &str = "\
You are writing a shift-change handoff for the next instance of a coding \
agent whose context is being fully reset. From the transcript digest, \
produce a dense structured markdown handoff with EXACTLY these sections: \
# Objective, # Current State, # Decisions Made, # Open Items, # Obligations \
(commands/tests that must stay green). Maximum 300 words. State facts only.";

pub fn handoff_prompt(transcript_digest: &str) -> String {
    format!(
        "Write the structured handoff for this session segment.\n\n<transcript>\n{transcript_digest}\n</transcript>"
    )
}

pub fn handoff_request(model: &str, prompt: String) -> ChatRequest {
    let mut req = ChatRequest::new(model);
    req.system = Some(HANDOFF_SYSTEM.to_string());
    req.messages = vec![vak_llm::Message::user_text(prompt)];
    req.max_tokens = 800;
    req.think = Some(false);
    req
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// One object per criterion, back to back, as `gemma4:e2b-mlx` does
    /// when it thinks first; and prose around a single object.
    #[test]
    fn verdicts_parse_from_concatenated_objects_and_prose() {
        let two = "```json\n{\"results\":[{\"criterion\":\"a\",\"verdict\":\"pass\",\"evidence\":\"x\"}]}\n\
                   {\"results\":[{\"criterion\":\"b\",\"verdict\":\"fail\",\"evidence\":\"y\"}]}\n```";
        let verdicts = parse_verdicts(two).unwrap();
        assert_eq!(verdicts.len(), 2);
        assert_eq!(verdicts[1].verdict, "fail");
        let prose = "Here is my verdict: {\"results\":[{\"criterion\":\"a\",\"verdict\":\"unknown\",\"evidence\":\"brace } in string\"}]} done";
        let verdicts = parse_verdicts(prose).unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].evidence, "brace } in string");
        assert!(parse_verdicts("no json here").is_err());
        assert!(parse_verdicts("{\"other\": 1}").is_err());
    }

    #[test]
    fn shell_criteria_detected_and_stripped() {
        assert!(is_shell_criterion("verify: cargo test --quiet"));
        assert!(is_shell_criterion("  verify:make lint"));
        assert!(!is_shell_criterion("the build must succeed"));
        assert_eq!(shell_command(" verify: cargo test "), "cargo test");
    }

    #[test]
    fn parses_clean_and_fenced_verdicts() {
        let clean = r#"{"results":[{"criterion":"tests","verdict":"pass","evidence":"green"},{"criterion":"lint","verdict":"fail","evidence":"3 warnings"}]}"#;
        let v = parse_verdicts(clean).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].criterion, "tests");
        assert_eq!(v[1].verdict, "fail");

        let fenced = format!("```json\n{clean}\n```");
        assert_eq!(parse_verdicts(&fenced).unwrap().len(), 2);

        let prose = "I think it passed but here: {\"results\":[]}";
        assert!(parse_verdicts(prose).is_err(), "empty results fail closed");
        assert!(parse_verdicts("no json at all").is_err());
    }

    #[test]
    fn digest_prefers_recent_turns_under_cap() {
        let msgs: Vec<vak_llm::Message> = (0..50)
            .map(|i| vak_llm::Message::user_text(format!("turn {i} filler filler filler")))
            .collect();
        let d = transcript_digest(&msgs, 400);
        assert!(d.contains("turn 49"), "most recent must survive");
        assert!(!d.contains("turn 0 "), "oldest should be cut");
    }
}
