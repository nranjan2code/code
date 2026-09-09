#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceIntent {
    NewRequest,
    Steering,
    Status,
    PauseSpeech,
    CancelRun,
    Unknown,
}

/// Conservative lexical classification for control phrases. It never grants
/// authority; ambiguous speech remains `Unknown` and is sent through the
/// normal prompt/approval path.
pub fn classify_control(text: &str) -> VoiceIntent {
    let value = text.trim().to_ascii_lowercase();
    if value.is_empty() {
        return VoiceIntent::Unknown;
    }
    if ["stop talking", "be quiet", "mute", "silence"]
        .iter()
        .any(|p| value == *p)
    {
        return VoiceIntent::PauseSpeech;
    }
    if ["cancel", "cancel that", "stop the task", "stop running"]
        .iter()
        .any(|p| value == *p)
    {
        return VoiceIntent::CancelRun;
    }
    if value.starts_with("what have you found") || value.starts_with("status") {
        return VoiceIntent::Status;
    }
    if value.starts_with("actually ") || value.starts_with("instead ") || value.starts_with("also ")
    {
        return VoiceIntent::Steering;
    }
    VoiceIntent::NewRequest
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_are_conservative_and_case_insensitive() {
        assert_eq!(classify_control("STOP TALKING"), VoiceIntent::PauseSpeech);
        assert_eq!(classify_control("cancel that"), VoiceIntent::CancelRun);
        assert_eq!(
            classify_control("Actually check the lockfile"),
            VoiceIntent::Steering
        );
        assert_eq!(
            classify_control("please inspect the build"),
            VoiceIntent::NewRequest
        );
    }
}
