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
            // The picker uses JSON strings for paths with spaces, punctuation
            // or escapes. Keep the quotes for the model and remove only @,
            // after checking the decoded full path against the workspace.
            if rest.starts_with('"') {
                let mut quoted = serde_json::Deserializer::from_str(rest).into_iter::<String>();
                if let Some(Ok(path)) = quoted.next() {
                    let length = quoted.byte_offset();
                    if !names_workspace_file(&root, &path) {
                        out.push('@');
                    }
                    out.push_str(&rest[..length]);
                    for _ in 0..rest[..length].chars().count() {
                        chars.next();
                    }
                    previous = Some('"');
                    continue;
                }
                // An unfinished/invalid quoted token has no reliable end.
                // Preserve it rather than resolving an @ inside its text.
                out.push_str(&text[index..]);
                break;
            }
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

    #[test]
    fn quoted_mentions_resolve_the_full_name_never_a_shorter_prefix() {
        let dir = workspace();
        let mut paths = vec!["space", "space name.txt", "नोट.txt", "a,b.txt"];
        if cfg!(unix) {
            paths.extend(["say \"hi\".txt", "back\\slash.txt", "trailing."]);
        }
        for path in paths {
            std::fs::write(dir.path().join(path), "x").unwrap();
            let quoted = serde_json::to_string(path).unwrap();
            let input = format!("read @{quoted} and @notes.txt.");
            assert_eq!(
                resolve_file_mentions(&input, dir.path()),
                format!("read {quoted} and notes.txt.")
            );
        }
        let missing = "read @\"space missing.txt\"";
        assert_eq!(resolve_file_mentions(missing, dir.path()), missing);
    }

    #[test]
    fn quoted_mentions_preserve_unknown_names_and_refuse_workspace_escapes() {
        let dir = workspace();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("escape name.txt"),
        )
        .unwrap();
        for path in [
            "../secret.txt",
            "escape name.txt",
            "src",
            "missing @notes.txt",
            outside.path().join("secret.txt").to_str().unwrap(),
        ] {
            let input = format!("@{}", serde_json::to_string(path).unwrap());
            assert_eq!(resolve_file_mentions(&input, dir.path()), input);
        }
        for input in [
            "@\"unfinished",
            "@\"bad\\q\"",
            "@\"bad @notes.txt",
            "mail@\"notes.txt\"",
        ] {
            assert_eq!(resolve_file_mentions(input, dir.path()), input);
        }
    }
}
