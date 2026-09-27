//! The workspace inbox (docs/design/72, "File in"): where a file a person
//! sends, on a channel or dropped in a client, is saved and named. Its bytes
//! never enter a prompt unless they are text small enough to inline; the
//! message carries `note`, which tells the model where the file is and
//! which reader understands it.

pub(crate) use vak_tools::INBOX_DIR;

/// The largest file a client may drop in. An Office package may inflate to
/// four times this within the reader's own bound.
pub(crate) const UPLOAD_MAX_BYTES: usize = 64 * 1024 * 1024;

/// The line that names a saved file to the model in place of its bytes.
pub(crate) fn note(filename: &str, saved: &str, bytes: &[u8]) -> String {
    let size = format!("{} KiB", bytes.len().div_ceil(1024));
    let text = std::str::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
        .is_some();
    // The path leads and is quoted as the argument to pass: a small model
    // given the sent name first called the reader with that name instead.
    if vak_ooxml::is_openxml_path(saved) || vak_pdf::is_pdf_path(saved) || vak_pdf::sniff(bytes) {
        format!(
            "[attached file at path \"{saved}\" ({size}, sent as '{filename}'). Read it with doc_read and that exact path; its contents are not in this message.]"
        )
    } else if text {
        format!(
            "[attached text file at path \"{saved}\" ({size}, sent as '{filename}'). Read it with read or doc_read and that exact path; its contents are not in this message.]"
        )
    } else {
        format!(
            "[attached file at path \"{saved}\" ({size}, sent as '{filename}'). It is not a text, PDF or Open XML file, so no reader here understands it yet; its bytes are not in this message.]"
        )
    }
}

/// Saves received bytes under `<workspace>/inbox/` with a digest-prefixed,
/// sanitised name. Returns the workspace-relative path. Never overwrites,
/// never follows a planted symlink out of the workspace, and saving the
/// same bytes under the same name twice yields the same file.
pub(crate) fn save_to_inbox(
    workspace: &std::path::Path,
    filename: &str,
    bytes: &[u8],
) -> Result<String, String> {
    use sha2::Digest as _;
    use std::io::Write as _;
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("workspace unavailable: {error}"))?;
    let inbox = workspace.join(INBOX_DIR);
    std::fs::create_dir_all(&inbox).map_err(|error| format!("inbox: {error}"))?;
    let inbox = inbox
        .canonicalize()
        .map_err(|error| format!("inbox: {error}"))?;
    if !inbox.starts_with(&workspace) {
        return Err("inbox resolves outside the workspace".into());
    }
    let digest = sha2::Sha256::digest(bytes);
    let prefix: String = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let name = format!("{prefix}-{}", sanitize_filename(filename));
    let path = inbox.join(&name);
    let relative = format!("{INBOX_DIR}/{name}");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("write failed: {error}"))?;
            Ok(relative)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing =
                std::fs::symlink_metadata(&path).map_err(|error| format!("inbox: {error}"))?;
            if existing.file_type().is_file() && std::fs::read(&path).ok().as_deref() == Some(bytes)
            {
                Ok(relative)
            } else {
                Err("a different file already has that name".into())
            }
        }
        Err(error) => Err(format!("create failed: {error}")),
    }
}

/// The last path component, with anything outside a conservative
/// character set replaced, no leading dots, and a bounded length.
pub(crate) fn sanitize_filename(filename: &str) -> String {
    let last = filename.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    let mut name: String = cleaned
        .chars()
        .rev()
        .take(120)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if name.is_empty() {
        name = "file".into();
    }
    name
}

/// A file already saved in `workspace`'s inbox, named by the path
/// `save_to_inbox` returned: its note for the model and what a client shows.
/// Refuses anything that is not a regular file directly inside the inbox.
pub(crate) fn attached(
    workspace: &std::path::Path,
    relative: &str,
) -> Result<(String, vak_session::AttachedFile), String> {
    let saved_name = relative
        .strip_prefix(INBOX_DIR)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|name| !name.is_empty() && !name.contains(['/', '\\']) && *name != "..")
        .ok_or_else(|| format!("{relative} is not a file in {INBOX_DIR}/"))?;
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("workspace unavailable: {error}"))?;
    let inbox = workspace
        .join(INBOX_DIR)
        .canonicalize()
        .map_err(|_| format!("{relative} was not found"))?;
    let path = inbox.join(saved_name);
    let metadata =
        std::fs::symlink_metadata(&path).map_err(|_| format!("{relative} was not found"))?;
    if !inbox.starts_with(&workspace) || !metadata.file_type().is_file() {
        return Err(format!("{relative} is not a file in {INBOX_DIR}/"));
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("{relative}: {error}"))?;
    let name = display_name(saved_name);
    let relative = format!("{INBOX_DIR}/{saved_name}");
    Ok((
        note(&name, &relative, &bytes),
        vak_session::AttachedFile {
            block: 0,
            path: relative,
            name,
            bytes: bytes.len() as u64,
        },
    ))
}

/// The name a saved file had before `save_to_inbox` prefixed its digest.
pub(crate) fn display_name(saved_name: &str) -> String {
    match saved_name.split_once('-') {
        Some((prefix, rest))
            if prefix.len() == 12 && prefix.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            rest.to_string()
        }
        _ => saved_name.to_string(),
    }
}
