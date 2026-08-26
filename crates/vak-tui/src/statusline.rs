//! Status bar and context display rendering.

use crate::data::ClientData;
use crate::render::Screen;
use crate::state::UiState;

pub async fn show_context(data: &ClientData, session_id: &str, ui: &UiState, screen: &mut Screen) {
    let (messages, total) = match data.transcript(session_id).await {
        Ok(t) => (t.count, t.usage.total_tokens()),
        Err(_) => (0, ui.total_in.saturating_add(ui.total_out)),
    };
    let window = u64::from(data.config().await.context_window.unwrap_or(0));
    let pct = if window > 0 {
        total.saturating_mul(100) / window.max(1)
    } else {
        0
    };
    let usage = if window > 0 {
        format!("{total} / {window} tokens · {}% of window", pct.min(999))
    } else {
        format!("~{total} tokens · window unknown")
    };
    screen.dim(&format!("next request: ~{messages} messages · {usage}",));
}
