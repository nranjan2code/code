//! Compatibility facade for Telegram's final plain-text retry.

pub fn strip_tags(html: &str) -> String {
    vak_delivery::telegram::strip_html(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_shared_codec() {
        assert_eq!(strip_tags("<b>ok</b>"), "ok");
    }
}
