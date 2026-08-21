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

pub struct ToolContext {
    pub cwd: PathBuf,
    pub cancel: CancellationToken,
    pub limits: OutputLimits,
}

impl ToolContext {
    pub fn new(cwd: PathBuf) -> Self {
        ToolContext {
            cwd,
            cancel: CancellationToken::new(),
            limits: OutputLimits::default(),
        }
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

        if rendered.len() <= self.limits.max_bytes {
            return rendered;
        }

        let budget = self.limits.max_bytes;
        let head_chars = (budget * 2 / 5).max(1);
        let tail_chars = (budget - head_chars).saturating_sub(120).max(1);
        let total_chars = rendered.chars().count();
        let head: String = rendered.chars().take(head_chars).collect();
        let tail: String = rendered
            .chars()
            .skip(total_chars.saturating_sub(tail_chars))
            .collect();

        let mut spill_note = String::new();
        if self.limits.spill_to_disk {
            let n = SPILL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let spill_path = std::env::temp_dir().join(format!("vakcoder-out-{n}.txt"));
            if std::fs::write(&spill_path, &rendered).is_ok() {
                spill_note = format!("\n[full output saved to {}]", spill_path.display());
            }
        }

        format!(
            "{head}\n[… {omitted} chars truncated …]{spill_note}\n{tail}",
            omitted = total_chars - head_chars - tail_chars
        )
    }
}

pub fn shared_ctx(dir: &Path) -> Arc<ToolContext> {
    Arc::new(ToolContext::new(dir.to_path_buf()))
}
