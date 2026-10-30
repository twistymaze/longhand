//! The AI providers that Longhand can send a summary request to (R7), and
//! the request route that Rig uses for each provider (KTD2).

use serde::{Deserialize, Serialize};

/// One AI provider. Each provider has a default address, except Custom,
/// whose address the user enters. The serialized name of each provider is
/// [`Provider::name`]: the settings, the Keychain account and the saved
/// summary progress use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
pub enum Provider {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "lm_studio")]
    LmStudio,
    /// An OpenAI-compatible server at an address that the user enters.
    #[serde(rename = "custom")]
    Custom,
}

impl Provider {
    /// All the providers, in the order of the provider list.
    pub const ALL: [Provider; 7] = [
        Self::OpenAi,
        Self::Anthropic,
        Self::Gemini,
        Self::OpenRouter,
        Self::Ollama,
        Self::LmStudio,
        Self::Custom,
    ];

    /// The usual address of the provider. `None` for Custom.
    pub fn default_address(self) -> Option<&'static str> {
        match self {
            Self::OpenAi => Some("https://api.openai.com/v1"),
            Self::Anthropic => Some("https://api.anthropic.com"),
            Self::Gemini => Some("https://generativelanguage.googleapis.com"),
            Self::OpenRouter => Some("https://openrouter.ai/api/v1"),
            Self::Ollama => Some("http://localhost:11434"),
            Self::LmStudio => Some("http://localhost:1234/v1"),
            Self::Custom => None,
        }
    }

    /// The name of the provider in the settings, in the Keychain account
    /// and in the saved summary progress. It is the serialized name.
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::OpenRouter => "openrouter",
            Self::Ollama => "ollama",
            Self::LmStudio => "lm_studio",
            Self::Custom => "custom",
        }
    }

    /// A cloud provider needs an API key, and the user does not enter its
    /// address (R8).
    pub fn is_cloud(self) -> bool {
        matches!(
            self,
            Self::OpenAi | Self::Anthropic | Self::Gemini | Self::OpenRouter
        )
    }

    /// A local provider runs on this computer and needs no API key (R8).
    pub fn is_local(self) -> bool {
        matches!(self, Self::Ollama | Self::LmStudio)
    }

    /// Whether the user can save an API key for this provider: the cloud
    /// providers, and Custom, whose key is optional (R8, R9).
    pub fn takes_key(self) -> bool {
        !self.is_local()
    }

    /// Whether the user can change the address of this provider: the local
    /// providers and Custom (R8, R9).
    pub fn address_editable(self) -> bool {
        !self.is_cloud()
    }

    /// The Thinking level of a provider entry that has no level from the
    /// user: Low for the local providers, Model default for the others
    /// (R6).
    pub fn default_thinking(self) -> ThinkingLevel {
        if self.is_local() {
            ThinkingLevel::Low
        } else {
            ThinkingLevel::ModelDefault
        }
    }

    /// How many requests go to the provider at the same time when the user
    /// set no number: 4 for the cloud providers, 1 for the local providers
    /// and Custom (R12).
    pub fn default_requests_at_once(self) -> u32 {
        if self.is_cloud() { 4 } else { 1 }
    }

    /// The kind of request that Rig sends to this provider.
    pub(crate) fn route(self) -> Route {
        match self {
            Self::OpenAi | Self::OpenRouter | Self::LmStudio | Self::Custom => Route::OpenAiChat,
            Self::Anthropic => Route::Anthropic,
            Self::Gemini => Route::Gemini,
            Self::Ollama => Route::Ollama,
        }
    }
}

/// How much the model thinks before it replies (R5). Model default sends
/// no reasoning control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    ModelDefault,
    Off,
    Low,
    Medium,
    High,
}

/// The kind of a summary request. Each kind has its own reply format and
/// its own signals for a cut-off reply and a prompt that is too long (KTD4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    /// OpenAI-compatible Chat Completions: `POST {address}/chat/completions`.
    OpenAiChat,
    /// Anthropic Messages: `POST /v1/messages`.
    Anthropic,
    /// Gemini `generateContent`.
    Gemini,
    /// Ollama's native `POST /api/chat`.
    Ollama,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_serialized_name_of_each_provider_is_its_name() {
        for provider in Provider::ALL {
            let json = serde_json::to_value(provider).unwrap();
            assert_eq!(json, serde_json::json!(provider.name()));
            assert_eq!(serde_json::from_value::<Provider>(json).unwrap(), provider);
        }
    }

    #[test]
    fn each_provider_is_cloud_local_or_custom() {
        for provider in Provider::ALL {
            let custom = provider == Provider::Custom;
            let kinds = [provider.is_cloud(), provider.is_local(), custom];
            assert_eq!(kinds.iter().filter(|&&b| b).count(), 1, "{provider:?}");
            assert_eq!(provider.takes_key(), !provider.is_local());
            assert_eq!(provider.address_editable(), !provider.is_cloud());
        }
    }

    #[test]
    fn each_provider_kind_has_its_own_defaults() {
        // Covers R6, R12.
        for provider in Provider::ALL {
            let (thinking, requests) = match provider {
                Provider::LmStudio | Provider::Ollama => (ThinkingLevel::Low, 1),
                Provider::Custom => (ThinkingLevel::ModelDefault, 1),
                _ => (ThinkingLevel::ModelDefault, 4),
            };
            assert_eq!(provider.default_thinking(), thinking, "{provider:?}");
            assert_eq!(
                provider.default_requests_at_once(),
                requests,
                "{provider:?}"
            );
        }
    }

    #[test]
    fn a_thinking_level_is_stored_by_its_snake_case_name() {
        let json = serde_json::to_value(ThinkingLevel::ModelDefault).unwrap();
        assert_eq!(json, serde_json::json!("model_default"));
        let low: ThinkingLevel = serde_json::from_value(serde_json::json!("low")).unwrap();
        assert_eq!(low, ThinkingLevel::Low);
    }
}
