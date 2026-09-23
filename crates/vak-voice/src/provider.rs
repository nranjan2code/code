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
    Local,
}

impl VoiceProvider {
    pub const ALL: [Self; 3] = [Self::Gemini, Self::OpenAi, Self::Local];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::OpenAi => "openai",
            Self::Local => "local",
        }
    }

    /// Credential variables consulted, in order, through the canonical
    /// secret chain. Empty for the offline provider.
    pub fn credential_vars(self) -> &'static [&'static str] {
        match self {
            Self::Gemini => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            Self::OpenAi => &["OPENAI_API_KEY"],
            Self::Local => &[],
        }
    }

    /// Encodings `/voice/speak` can return for this provider.
    pub fn speak_formats(self) -> &'static [SpeakFormat] {
        match self {
            Self::Gemini => &[SpeakFormat::Wav],
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
        assert_eq!(names, vec!["gemini", "openai", "local"]);
        let json = serde_json::to_string(&catalogue()).unwrap();
        assert!(json.contains("GEMINI_API_KEY"));
        assert!(!json.contains("KEY="));
    }
}
