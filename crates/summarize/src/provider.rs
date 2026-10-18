//! The AI providers that Longhand can send a summary request to (R7), and
//! the request route that Rig uses for each provider (KTD2).

/// One AI provider. Each provider has a default address, except Custom,
/// whose address the user enters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    OpenAi,
    Anthropic,
    Gemini,
    OpenRouter,
    Ollama,
    LmStudio,
    /// An OpenAI-compatible server at an address that the user enters.
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
