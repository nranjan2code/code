//! Reflection stage (docs/design/26-learning.md L1): after a clean run, an
//! auxiliary model call may propose durable knowledge. Everything is
//! filtered through dedup against existing MEMORY.md and hard caps — noise
//! is the failure mode of self-improving systems, so the bar for writing is
//! deliberately high.
//!
//! Skill drafts from reflection land in the same human-gated review queue
//! as `propose_skill`; promotion is never automatic.

use vak_llm::types::ChatRequest;
use vak_llm::{ContentBlock, Message, Role};

use crate::memory;

/// At most this many notes per reflection — reflection is a filter, not a
/// firehose.
pub const MAX_NOTES_PER_RUN: usize = 2;
/// Token-set Jaccard at or above this means "already known".
const DEDUP_THRESHOLD: f32 = 0.55;
/// Transcript tail fed to the reflector, in messages.
const TAIL_MESSAGES: usize = 24;

#[derive(Debug, Clone, PartialEq)]
pub struct NoteProposal {
    pub note: String,
    pub kind: String,
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillDraft {
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Debug, Clone, Default)]
pub struct Proposals {
    pub notes: Vec<NoteProposal>,
    pub skill: Option<SkillDraft>,
}

fn tokens(text: &str) -> std::collections::HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 3)
        .map(str::to_string)
        .collect()
}

/// Jaccard similarity over word sets — cheap, deterministic, good enough to
/// catch "the deploy script pauses before rollback" twice phrased.
pub fn jaccard(a: &str, b: &str) -> f32 {
    let (a, b) = (tokens(a), tokens(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(&b).count() as f32;
    let union = a.union(&b).count() as f32;
    inter / union
}

/// Extract the first balanced JSON object from a model reply. Reflection
/// replies are small; scanning beats demanding perfect formatting.
fn extract_json(reply: &str) -> Option<&str> {
    let start = reply.find('{')?;
    let bytes = reply.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes[start..].iter().enumerate() {
        if escaped {
            escaped = false;
        } else {
            match b {
                b'"' => in_string = !in_string,
                b'\\' if in_string => escaped = true,
                _ => {}
            }
        }
        if !in_string {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&reply[start..=start + i]);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

pub fn parse_proposals(reply: &str) -> Proposals {
    let Some(json) = extract_json(reply) else {
        return Proposals::default();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Proposals::default();
    };
    let mut out = Proposals::default();

    if let Some(notes) = v["notes"].as_array() {
        for n in notes {
            let note = n["note"].as_str().unwrap_or("").trim().to_string();
            if note.len() < 8 {
                continue;
            }
            out.notes.push(NoteProposal {
                note,
                kind: match n["kind"].as_str().unwrap_or("fact") {
                    "decision" | "preference" | "reference" => {
                        n["kind"].as_str().unwrap_or("fact").to_string()
                    }
                    _ => "fact".into(),
                },
                tag: n["tag"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect(),
            });
            // Cap applies to VALID proposals, not raw attempts.
            if out.notes.len() >= MAX_NOTES_PER_RUN {
                break;
            }
        }
    }

    if let Some(skill) = v["skill"].as_object()
        && let Some(name) = crate::learning::sanitize_name(skill["name"].as_str().unwrap_or(""))
    {
        let desc = skill["description"].as_str().unwrap_or("").trim();
        let instr = skill["instructions"].as_str().unwrap_or("").trim();
        if !desc.is_empty() && !instr.is_empty() {
            out.skill = Some(SkillDraft {
                name,
                description: desc.to_string(),
                instructions: instr.to_string(),
            });
        }
    }
    out
}

fn system_prompt() -> String {
    "You are the reflection stage of a coding agent. Given a recent \
     conversation, decide what is worth persisting across future sessions. \
     Be extremely selective: only durable decisions, facts or preferences — \
     not task chatter. Reply with ONLY minified JSON of shape \
     {\"notes\":[{\"note\":\"...\",\"kind\":\"fact|decision|preference|reference\",\"tag\":\"kebab-tag\"}],\
     \"skill\":{\"name\":\"kebab-name\",\"description\":\"one line\",\
     \"instructions\":\"markdown\"}} — at most 2 notes; omit \"notes\" or \
     \"skill\" when nothing qualifies. Reply {} when nothing is worth keeping."
        .to_string()
}

/// Run one reflection pass against `provider` over the given transcript.
/// Returns proposals parsed from the model's reply (not yet written).
pub async fn propose(
    provider: std::sync::Arc<dyn vak_llm::Provider>,
    model: &str,
    transcript_tail: &str,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<Proposals, String> {
    let tail: String = transcript_tail
        .lines()
        .rev()
        .take(TAIL_MESSAGES * 4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let user = Message {
        role: Role::User,
        content: vec![ContentBlock::text(format!(
            "Recent conversation:\n\n{}",
            &tail[..tail.len().min(12_000)]
        ))],
    };
    let mut req = ChatRequest::new(model);
    req.system = Some(system_prompt());
    req.messages = vec![user];
    req.max_tokens = 700;

    let stream = provider
        .stream(req, cancel)
        .await
        .map_err(|e| format!("reflection call failed: {e}"))?;
    let reply = stream
        .result()
        .await
        .map_err(|e| format!("reflection call failed: {e}"))?
        .text_content();

    Ok(parse_proposals(&reply))
}

/// Apply proposals: dedup notes against existing memory (Jaccard), write
/// survivors, queue any skill draft. Returns counts.
pub fn apply(
    home: &std::path::Path,
    cwd: &std::path::Path,
    session_id: &str,
    proposals: &Proposals,
) -> Result<(usize, bool), String> {
    let existing: Vec<String> = memory::list_notes(home, cwd)
        .into_iter()
        .map(|n| n.text)
        .collect();
    let mut written = 0usize;
    for n in &proposals.notes {
        let duplicate = existing
            .iter()
            .any(|known| jaccard(known, &n.note) >= DEDUP_THRESHOLD);
        if duplicate {
            continue;
        }
        memory::append_note(home, cwd, &n.kind, &n.tag, session_id, &n.note)?;
        written += 1;
    }
    let mut queued = false;
    if let Some(sk) = &proposals.skill {
        // Route through the same review queue as the propose_skill tool by
        // synthesizing its exact output contract.
        let id = uuid::Uuid::now_v7().simple().to_string();
        let dir = home.join("skill-proposals").join(memory::hash_cwd(cwd));
        std::fs::create_dir_all(&dir).map_err(|e| format!("create proposals dir: {e}"))?;
        let body = format!(
            "---\nname: \"{name}\"\ndescription: \"{desc}\"\n---\n\n{instr}\n\n<!-- proposed-by: {sid} at {ts}; proposal id {id}; source: reflection -->\n",
            name = sk.name,
            desc = sk.description.replace('"', "'"),
            instr = sk.instructions,
            sid = session_id,
            ts = chrono::Utc::now().to_rfc3339(),
        );
        std::fs::write(dir.join(format!("{id}.md")), body)
            .map_err(|e| format!("write proposal: {e}"))?;
        queued = true;
    }
    Ok((written, queued))
}
