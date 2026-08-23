pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Spinner frame; reduced-motion builds render a static glyph.
pub fn frame(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
}

pub fn frame_motion(tick: usize, animated: bool) -> &'static str {
    if animated { frame(tick) } else { "·" }
}

pub fn fmt_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn fmt_tokens(n: u64) -> String {
    if n < 1_000 {
        return format!("{n}");
    }
    let (value, suffix) = if n < 1_000_000 {
        (n as f64 / 1_000.0, "k")
    } else {
        (n as f64 / 1_000_000.0, "M")
    };
    let mut s = format!("{value:.1}");
    if s.ends_with(".0") {
        s.truncate(s.len() - 2);
    }
    format!("{s}{suffix}")
}
