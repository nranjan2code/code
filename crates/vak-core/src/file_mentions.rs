//! `@path` file mentions typed in the composer.
//!
//! The picker only completes a name; the model would otherwise receive the
//! literal `@notes.txt` and pass it to a tool as a path. A mention is
//! rewritten to the bare path only when it starts a word and names an existing
//! file whose canonical path is inside the canonical workspace, so an email
//! address, an unknown name, a traversal or a symlink out of the workspace is
//! left exactly as typed (invariant 10).

use std::path::Path;

const PATH_CHARS: &str = "._/-";

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || PATH_CHARS.contains(c)
}

fn names_workspace_file(root: &Path, candidate: &str) -> bool {
    if candidate.is_empty() || candidate.starts_with('/') {
        return false;
    }
    let Ok(resolved) = root.join(candidate).canonicalize() else {
        return false;
    };
    resolved.is_file() && resolved.starts_with(root)
}

pub fn resolve_file_mentions(text: &str, workspace: &Path) -> String {
    if !text.contains('@') {
        return text.to_string();
    }
    let Ok(root) = workspace.canonicalize() else {
        return text.to_string();
    };
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let mut previous: Option<char> = None;
    while let Some((index, c)) = chars.next() {
        let starts_word = previous.is_none_or(|p| p.is_whitespace() || p == '(');
        if c == '@' && starts_word {
            let rest = &text[index + 1..];
            let token_len = rest
                .char_indices()
                .find(|(_, ch)| !is_path_char(*ch))
                .map_or(rest.len(), |(i, _)| i);
            let token = &rest[..token_len];
            let trimmed = token.trim_end_matches('.');
            if names_workspace_file(&root, trimmed) {
                out.push_str(trimmed);
                for _ in 0..trimmed.chars().count() {
                    chars.next();
                }
                previous = trimmed.chars().last();
                continue;
            }
        }
        out.push(c);
        previous = Some(c);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "x").unwrap();
        dir
    }

    #[test]
    fn existing_file_loses_its_at_sign() {
        let dir = workspace();
        assert_eq!(
            resolve_file_mentions("read @notes.txt and (@src/main.rs).", dir.path()),
            "read notes.txt and (src/main.rs)."
        );
        assert_eq!(
            resolve_file_mentions("@notes.txt.", dir.path()),
            "notes.txt."
        );
    }

    #[test]
    fn email_unknown_and_mid_word_mentions_are_untouched() {
        let dir = workspace();
        for text in [
            "mail me@notes.txt now",
            "ask @alice about it",
            "see @missing.txt",
            "a@b.com",
            "@",
            "@@notes.txt",
        ] {
            assert_eq!(resolve_file_mentions(text, dir.path()), text);
        }
    }

    #[test]
    fn traversal_absolute_and_symlink_escapes_are_untouched() {
        let dir = workspace();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("link.txt"),
        )
        .unwrap();
        let absolute = format!("@{}", outside.path().join("secret.txt").display());
        for text in ["@../secret.txt", "@link.txt", absolute.as_str()] {
            assert_eq!(resolve_file_mentions(text, dir.path()), text);
        }
    }

    #[test]
    fn directories_are_not_files() {
        let dir = workspace();
        assert_eq!(resolve_file_mentions("@src", dir.path()), "@src");
    }
}
