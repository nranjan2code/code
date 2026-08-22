//! @file mention expansion: `@path` tokens in the prompt attach file
//! contents to the submitted message. Pure and unit-tested; the expanded
//! text becomes the user message, so model-visible stays logged.

use std::path::Path;

/// Max bytes read per mentioned file; larger files are noted, not inlined.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Mentions resolved per submit; keeps pathological prompts bounded.
const MAX_MENTIONS: usize = 8;

pub struct Expanded {
    pub prompt: String,
    pub attached: Vec<String>,
    pub missing: Vec<String>,
}

/// Extracts `@token` paths (whitespace-delimited) from `text`.
pub fn mention_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, token) in text.split_whitespace().enumerate() {
        let _ = i;
        let Some(rest) = token.strip_prefix('@') else {
            continue;
        };
        if rest.is_empty() || out.len() >= MAX_MENTIONS {
            continue;
        }
        // Strip common trailing punctuation from prose contexts.
        let cleaned = rest.trim_end_matches(&['.', ',', ';', ':', ')', ']'][..]);
        if !cleaned.is_empty() {
            out.push(cleaned.to_string());
        }
    }
    out
}

/// Expands every resolvable mention into a fenced attachment appended to
/// the prompt. Unresolvable tokens stay literal and are reported missing.
pub fn expand(text: &str, cwd: &Path) -> Expanded {
    let mut attached = Vec::new();
    let mut missing = Vec::new();
    let mut body = String::new();
    for p in mention_paths(text) {
        let full = cwd.join(&p);
        match std::fs::metadata(&full) {
            Ok(m) if m.is_file() && m.len() <= MAX_FILE_BYTES => {
                match std::fs::read_to_string(&full) {
                    Ok(content) => {
                        attached.push(p.clone());
                        body.push_str(&format!("\n\n---\nAttached file: {p}\n```\n{content}\n```"));
                    }
                    Err(_) => missing.push(p),
                }
            }
            _ => missing.push(p),
        }
    }
    let prompt = if body.is_empty() {
        text.to_string()
    } else {
        format!("{text}{body}")
    };
    Expanded {
        prompt,
        attached,
        missing,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn no_mentions_leaves_prompt_untouched() {
        let e = expand("just a normal prompt", Path::new("."));
        assert_eq!(e.prompt, "just a normal prompt");
        assert!(e.attached.is_empty() && e.missing.is_empty());
    }

    #[test]
    fn existing_file_is_attached_with_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hello facts").unwrap();
        let e = expand("summarize @notes.txt please", dir.path());
        assert_eq!(e.attached, vec!["notes.txt".to_string()]);
        assert!(e.prompt.contains("Attached file: notes.txt"));
        assert!(e.prompt.contains("hello facts"));
        assert!(e.prompt.starts_with("summarize @notes.txt please"));
    }

    #[test]
    fn missing_and_directory_mentions_report_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        let e = expand("look at @nope.txt and @subdir", dir.path());
        assert_eq!(
            e.missing,
            vec!["nope.txt".to_string(), "subdir".to_string()]
        );
        assert_eq!(e.prompt, "look at @nope.txt and @subdir");
    }

    #[test]
    fn trailing_punctuation_is_trimmed_from_tokens() {
        let paths = mention_paths("see @a.rs, then @b.rs.");
        assert_eq!(paths, vec!["a.rs".to_string(), "b.rs".to_string()]);
    }

    #[test]
    fn lone_at_sign_is_ignored() {
        assert!(mention_paths("email me @ or @ ok").is_empty());
    }

    #[test]
    fn nested_relative_paths_resolve_against_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn f() {}").unwrap();
        let e = expand("review @src/lib.rs", dir.path());
        assert_eq!(e.attached, vec!["src/lib.rs".to_string()]);
        assert!(e.prompt.contains("fn f() {}"));
    }
}
