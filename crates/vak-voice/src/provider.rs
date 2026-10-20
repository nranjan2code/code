//! The closed set of voice providers and their static capability catalogue.
//!
//! A provider name is parsed exactly once, here. Every surface — the socket,
//! channel voice notes, `/voice/speak`, `/voice/providers`, doctor — resolves
//! through [`VoiceProvider`], so they cannot disagree about which names exist
//! or what an unset route means. Model and voice ids are never listed here:
//! they are properties of the operator's key (invariant 9).

use crate::SpeakFormat;
use serde::Serialize;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceProvider {
    Gemini,
    OpenAi,
    OpenRouter,
    Local,
}

impl VoiceProvider {
    pub const ALL: [Self; 4] = [Self::Gemini, Self::OpenAi, Self::OpenRouter, Self::Local];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::Local => "local",
        }
    }

    /// Credential variables consulted, in order, through the canonical
    /// secret chain. Empty for the offline provider.
    pub fn credential_vars(self) -> &'static [&'static str] {
        match self {
            Self::Gemini => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            Self::OpenAi => &["OPENAI_API_KEY"],
            Self::OpenRouter => &["OPENROUTER_API_KEY"],
            Self::Local => &[],
        }
    }

    /// Encodings `/voice/speak` can return for this provider.
    pub fn speak_formats(self) -> &'static [SpeakFormat] {
        match self {
            Self::Gemini => &[SpeakFormat::Wav],
            Self::OpenRouter => &[SpeakFormat::Wav, SpeakFormat::Mp3],
            Self::OpenAi | Self::Local => &[
                SpeakFormat::Wav,
                SpeakFormat::Pcm16,
                SpeakFormat::OggOpus,
                SpeakFormat::Mp3,
            ],
        }
    }

    /// The effective route, or an actionable reason there is none. An unset
    /// provider is a configuration gap, never an implicit vendor choice.
    pub fn resolve(configured: Option<&str>) -> Result<Self, String> {
        match configured.map(str::trim).filter(|name| !name.is_empty()) {
            None => Err("Choose a voice provider in Voice settings".into()),
            Some(name) => name.parse(),
        }
    }
}

/// Account-discovered choices for the implemented batch speech endpoints.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VoiceModels {
    pub transcription: Vec<String>,
    pub synthesis: Vec<String>,
    pub conversation_pack: Option<ConversationPack>,
}

/// A suggested batch conversation pair drawn entirely from a live catalogue.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationPack {
    pub provider: &'static str,
    pub transcription_model: String,
    pub synthesis_model: String,
    pub voice_name: Option<String>,
}

impl VoiceProvider {
    /// Classify provider model families, not arbitrary non-TTS catalogue entries.
    /// Unknown ids remain usable through an operator's explicit configuration.
    pub fn model_choices(self, models: &[String]) -> VoiceModels {
        let mut choices = VoiceModels::default();
        for model in models {
            let id = model.to_ascii_lowercase();
            let segments: Vec<_> = id.split('-').collect();
            let streaming = segments.contains(&"live")
                || segments.contains(&"realtime")
                || segments.contains(&"audio");
            let synthesis = segments.contains(&"tts");
            let transcription = match self {
                Self::OpenAi => {
                    !streaming
                        && !segments.contains(&"diarize")
                        && (segments.contains(&"transcribe") || id.starts_with("whisper-"))
                }
                Self::Gemini => {
                    !streaming
                        && (segments.contains(&"transcribe")
                            || (id.starts_with("gemini-")
                                && (id.ends_with("-flash")
                                    || id.ends_with("-flash-lite")
                                    || id.ends_with("-pro"))))
                }
                Self::Local | Self::OpenRouter => false,
            };
            if transcription {
                choices.transcription.push(model.clone());
            }
            if synthesis && matches!(self, Self::Gemini | Self::OpenAi) {
                choices.synthesis.push(model.clone());
            }
        }
        // A suggestion is a convenience, never a claim of tested access. Prefer
        // stable aliases, dedicated transcription and compact speech families;
        // within a family the provider's versioned ids sort newest first.
        let stable = |id: &str| {
            !id.contains("preview")
                && !id
                    .split('-')
                    .any(|part| part.len() >= 4 && part.chars().all(|c| c.is_ascii_digit()))
        };
        choices.transcription.sort_by_key(|id| {
            (
                std::cmp::Reverse(stable(id)),
                std::cmp::Reverse(id.ends_with("-transcribe")),
                std::cmp::Reverse(id.clone()),
            )
        });
        choices.synthesis.sort_by_key(|id| {
            (
                std::cmp::Reverse(stable(id)),
                std::cmp::Reverse(id.split('-').any(|part| part == "mini" || part == "lite")),
                std::cmp::Reverse(id.clone()),
            )
        });
        choices.conversation_pack = choices
            .transcription
            .first()
            .zip(choices.synthesis.first())
            .map(|(listen, speak)| ConversationPack {
                provider: self.as_str(),
                transcription_model: listen.clone(),
                synthesis_model: speak.clone(),
                voice_name: None,
            });
        choices
    }
}

impl FromStr for VoiceProvider {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.as_str() == value)
            .ok_or_else(|| {
                format!(
                    "unknown voice provider '{value}'; expected one of {}",
                    Self::ALL.map(Self::as_str).join(", ")
                )
            })
    }
}

impl fmt::Display for VoiceProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Static capability description served by `/voice/providers`. Contains no
/// credential values and no model ids.
#[derive(Debug, Clone, Serialize)]
pub struct VoiceDescriptor {
    pub name: &'static str,
    pub credential_vars: &'static [&'static str],
    pub formats: &'static [SpeakFormat],
}

pub fn catalogue() -> Vec<VoiceDescriptor> {
    VoiceProvider::ALL
        .into_iter()
        .map(|provider| VoiceDescriptor {
            name: provider.as_str(),
            credential_vars: provider.credential_vars(),
            formats: provider.speak_formats(),
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_nothing_else_parses() {
        for provider in VoiceProvider::ALL {
            assert_eq!(provider.as_str().parse::<VoiceProvider>(), Ok(provider));
        }
        for alias in ["google", "gemini-live", "google-live", "OpenAI", ""] {
            assert!(alias.parse::<VoiceProvider>().is_err(), "{alias}");
        }
    }

    #[test]
    fn packs_only_use_discovered_batch_models() {
        let models = [
            "gpt-transcribe",
            "gpt-live-transcribe",
            "gpt-realtime-whisper",
            "gpt-4o-transcribe-diarize",
            "gpt-4o-mini-tts",
            "gpt-4o-mini-tts-2025-12-15",
        ]
        .map(str::to_string);
        let choices = VoiceProvider::OpenAi.model_choices(&models);
        assert_eq!(choices.transcription, ["gpt-transcribe"]);
        let pack = choices.conversation_pack.unwrap();
        assert_eq!(pack.transcription_model, "gpt-transcribe");
        assert_eq!(pack.synthesis_model, "gpt-4o-mini-tts");
        assert!(
            VoiceProvider::OpenAi
                .model_choices(&[])
                .conversation_pack
                .is_none()
        );
        assert!(
            VoiceProvider::OpenAi
                .model_choices(&models[..1])
                .conversation_pack
                .is_none()
        );
    }

    #[test]
    fn gemini_choices_exclude_unrelated_and_streaming_models() {
        let models = [
            "gemini-3.5-transcribe",
            "gemini-3.5-transcribe-live",
            "veo-video",
            "lyria-music",
            "gemini-3.8-live",
            "gemini-3.8-flash-image",
            "aqa",
            "gemini-3.8-flash",
            "gemini-3.8-flash-tts",
            "gemini-3.8-flash-lite-tts",
        ]
        .map(str::to_string);
        let choices = VoiceProvider::Gemini.model_choices(&models);
        assert_eq!(
            choices.transcription,
            ["gemini-3.5-transcribe", "gemini-3.8-flash"]
        );
        let pack = choices.conversation_pack.unwrap();
        assert_eq!(pack.synthesis_model, "gemini-3.8-flash-lite-tts");
        let future = ["gemini-9.9-transcribe", "gemini-9.9-flash-lite-tts"].map(str::to_string);
        assert_eq!(
            VoiceProvider::Gemini
                .model_choices(&future)
                .conversation_pack
                .unwrap()
                .transcription_model,
            future[0]
        );
    }

    #[test]
    fn an_unset_route_is_a_gap_not_a_default() {
        assert!(VoiceProvider::resolve(None).is_err());
        assert!(VoiceProvider::resolve(Some("  ")).is_err());
        assert_eq!(
            VoiceProvider::resolve(Some(" openai ")),
            Ok(VoiceProvider::OpenAi)
        );
    }

    #[test]
    fn catalogue_names_every_provider_and_no_secret_values() {
        let names: Vec<_> = catalogue().iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["gemini", "openai", "openrouter", "local"]);
        let json = serde_json::to_string(&catalogue()).unwrap();
        assert!(json.contains("GEMINI_API_KEY"));
        assert!(!json.contains("KEY="));
    }
}
