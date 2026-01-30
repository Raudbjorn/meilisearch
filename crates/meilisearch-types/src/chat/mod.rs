//! Shared chat completion types for the Meilisearch chat feature.
//!
//! This module provides core types used across the chat completion implementation,
//! including provider configuration, prompts, and settings. These types are
//! HTTP-independent and can be used by both the server routes and library clients.
//!
//! # Module Structure
//!
//! - [`config`]: Provider-specific configuration types (Anthropic, OpenAI, Azure, etc.)
//! - Re-exports from [`crate::features`]: Core settings and prompt types
//!
//! # Example
//!
//! ```
//! use meilisearch_types::chat::{AnthropicConfig, ChatCompletionSource};
//!
//! // Create Anthropic configuration
//! let config = AnthropicConfig::new("sk-ant-api-key".to_string());
//! assert_eq!(config.base_url, AnthropicConfig::DEFAULT_BASE_URL);
//! ```

pub mod config;

// Re-export config types at module level for convenience
pub use config::{
    AnthropicConfig, AnthropicConfigError, AzureOpenAiConfig, ChatProviderConfig,
    OpenAiCompatibleConfig,
};

// Re-export core chat types from features module for backwards compatibility
// and to provide a dedicated chat-focused module path.
pub use crate::features::{
    ChatCompletionPrompts, ChatCompletionSettings, ChatCompletionSource, SystemRole,
    DEFAULT_CHAT_SEARCH_DESCRIPTION_PROMPT, DEFAULT_CHAT_SEARCH_FILTER_PARAM_PROMPT,
    DEFAULT_CHAT_SEARCH_INDEX_UID_PARAM_PROMPT, DEFAULT_CHAT_SEARCH_Q_PARAM_PROMPT,
    DEFAULT_CHAT_SYSTEM_PROMPT,
};

/// Chat workspace identifier type alias.
///
/// A workspace is a named container for chat completion settings and conversations.
pub type WorkspaceUid = String;

/// Maximum number of tool call iterations allowed in a single chat completion request.
///
/// This prevents infinite loops when the LLM keeps requesting tool calls.
pub const MAX_TOOL_CALL_ITERATIONS: usize = 20;

/// Internal function names used for Meilisearch-specific tool calls.
pub mod function_names {
    /// Function name to report search progress to the frontend.
    ///
    /// This function is used to report what Meilisearch is doing, allowing
    /// the frontend to display progress indicators.
    pub const MEILI_SEARCH_PROGRESS: &str = "_meiliSearchProgress";

    /// Function name to append a conversation message.
    ///
    /// This function is used to append a conversation message in the user
    /// conversation, keeping context for follow-up questions.
    pub const MEILI_APPEND_CONVERSATION_MESSAGE: &str = "_meiliAppendConversationMessage";

    /// Function name to report search sources to the frontend.
    ///
    /// The call ID is associated with the one used by the search progress function.
    pub const MEILI_SEARCH_SOURCES: &str = "_meiliSearchSources";

    /// Internal function name for the LLM to search in indexes.
    ///
    /// This function should not be exposed to end users as the LLM calls it
    /// and Meilisearch handles the actual search execution.
    pub const MEILI_SEARCH_IN_INDEX: &str = "_meiliSearchInIndex";
}

/// Parameters for the internal search function called by the LLM.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SearchInIndexParameters {
    /// The index UID to search in.
    pub index_uid: String,
    /// The query parameter to use.
    #[serde(default)]
    pub q: Option<String>,
    /// The filter parameter to use.
    #[serde(default)]
    pub filter: Option<String>,
}

/// Tracks which internal functions are enabled for a chat session.
///
/// These functions are used to communicate between Meilisearch and the frontend,
/// providing progress updates, sources, and conversation context.
#[derive(Default, Debug, Clone, Copy)]
pub struct FunctionSupport {
    /// Whether the `_meiliSearchProgress` function is enabled.
    ///
    /// When enabled, the frontend will be informed about what searches
    /// are being performed.
    pub report_progress: bool,

    /// Whether the `_meiliSearchSources` function is enabled.
    ///
    /// When enabled, the frontend will receive information about the
    /// sources (documents) that contributed to the response.
    pub report_sources: bool,

    /// Whether the `_meiliAppendConversationMessage` function is enabled.
    ///
    /// When enabled, messages will be appended to the conversation context
    /// to maintain context for follow-up questions.
    pub append_to_conversation: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_in_index_parameters_deserialize() {
        let json = r#"{"index_uid": "movies", "q": "action", "filter": "year > 2000"}"#;
        let params: SearchInIndexParameters = serde_json::from_str(json).unwrap();
        assert_eq!(params.index_uid, "movies");
        assert_eq!(params.q, Some("action".to_string()));
        assert_eq!(params.filter, Some("year > 2000".to_string()));
    }

    #[test]
    fn test_search_in_index_parameters_minimal() {
        let json = r#"{"index_uid": "products"}"#;
        let params: SearchInIndexParameters = serde_json::from_str(json).unwrap();
        assert_eq!(params.index_uid, "products");
        assert_eq!(params.q, None);
        assert_eq!(params.filter, None);
    }

    #[test]
    fn test_function_support_default() {
        let support = FunctionSupport::default();
        assert!(!support.report_progress);
        assert!(!support.report_sources);
        assert!(!support.append_to_conversation);
    }
}
