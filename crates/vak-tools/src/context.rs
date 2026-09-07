use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct OutputLimits {
    pub max_bytes: usize,
    pub max_line_chars: usize,
    pub spill_to_disk: bool,
}

impl Default for OutputLimits {
    fn default() -> Self {
        OutputLimits {
            max_bytes: 30_000,
            max_line_chars: 2_000,
            spill_to_disk: true,
        }
    }
}

static SPILL_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub cancel: CancellationToken,
    pub limits: OutputLimits,
    pub sandbox: Option<Arc<dyn crate::sandbox::Sandbox>>,
    pub sandbox_sink: Option<crate::sandbox_events::SandboxEventSink>,
}

impl ToolContext {
    pub fn new(cwd: PathBuf) -> Self {
        ToolContext {
            cwd,
            cancel: CancellationToken::new(),
            limits: OutputLimits::default(),
            sandbox: None,
            sandbox_sink: None,
        }
    }

    pub fn with_sandbox_sink(mut self, sink: crate::sandbox_events::SandboxEventSink) -> Self {
        self.sandbox_sink = Some(sink);
        self
    }

    pub fn resolve(&self, p: &Path) -> PathBuf {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.cwd.join(p)
        }
    }

    pub fn truncate_output(&self, out: String) -> String {
        let mut rendered = String::with_capacity(out.len());
        for line in out.split_inclusive('\n') {
            let char_len = line.chars().count();
            if char_len > self.limits.max_line_chars {
                let kept: String = line.chars().take(self.limits.max_line_chars).collect();
                let had_newline = kept.ends_with('\n');
                rendered.push_str(kept.trim_end_matches(['\r', '\n']));
                rendered.push_str("… [line truncated]");
                if had_newline {
                    rendered.push('\n');
                }
            } else {
                rendered.push_str(line);
            }
        }

        let total_chars = rendered.chars().count();
        if total_chars <= self.limits.max_bytes {
            return rendered;
        }

        // All arithmetic in chars: the byte length of multibyte content can
        // exceed the budget while the char count does not.
        let budget = self.limits.max_bytes;
        let head_chars = (budget * 2 / 5).max(1);
        let tail_chars = (budget - head_chars).saturating_sub(120).max(1);
        let head: String = rendered.chars().take(head_chars).collect();
        let tail: String = rendered
            .chars()
            .skip(total_chars.saturating_sub(tail_chars))
            .collect();

        let mut spill_note = String::new();
        if self.limits.spill_to_disk {
            let n = SPILL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Some(spill_path) = write_spill_file(n, &rendered) {
                spill_note = format!("\n[full output saved to {}]", spill_path.display());
            }
        }

        format!(
            "{head}\n[… {omitted} chars truncated …]{spill_note}\n{tail}",
            omitted = total_chars.saturating_sub(head_chars + tail_chars)
        )
    }
}

impl Default for ToolContext {
    fn default() -> Self {
        Self::new(PathBuf::new())
    }
}

/// Spill files live in a per-process directory created with 0700 so other
/// local users cannot read truncated tool output; file contents are 0600.
fn write_spill_file(n: u64, content: &str) -> Option<PathBuf> {
    static SPILL_DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    let dir = SPILL_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("vak-spill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        Some(dir)
    });
    let dir = dir.as_ref()?;
    let path = dir.join(format!("out-{n}.txt"));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(&path).ok()?.write_all(content.as_bytes()).ok()?;
    Some(path)
}

pub fn shared_ctx(dir: &Path) -> Arc<ToolContext> {
    Arc::new(ToolContext::new(dir.to_path_buf()))
}
