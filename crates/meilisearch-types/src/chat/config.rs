//! Shared chat provider configuration types.
//!
//! This module contains configuration types that are HTTP-independent and can be
//! used by both the meilisearch HTTP server and the embedded meilisearch-lib.

use crate::error::{Code, ResponseError};
use crate::features::{ChatCompletionSettings, ChatCompletionSource};
use serde::{Deserialize, Serialize};

// ============================================================================
// Anthropic Configuration
// ============================================================================

/// Configuration for the Anthropic API client.
///
/// This struct holds the necessary credentials and settings to communicate
/// with Anthropic's Messages API.
///
/// # Example
///
/// ```
/// use meilisearch_types::chat::AnthropicConfig;
///
/// let config = AnthropicConfig::new("sk-ant-api-key".to_string());
/// assert_eq!(config.base_url, AnthropicConfig::DEFAULT_BASE_URL);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    /// The Anthropic API key for authentication.
    pub api_key: String,
    /// The base URL for the Anthropic API.
    pub base_url: String,
    /// The Anthropic API version to use.
    pub anthropic_version: String,
}

/// Error type for Anthropic configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnthropicConfigError {
    /// API key is required for Anthropic source.
    MissingApiKey,
    /// Settings source is not Anthropic.
    WrongSource,
}

impl std::fmt::Display for AnthropicConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingApiKey => write!(f, "API key is required for Anthropic source"),
            Self::WrongSource => write!(f, "Settings source must be Anthropic"),
        }
    }
}

impl std::error::Error for AnthropicConfigError {}

impl AnthropicConfig {
    /// Default Anthropic API base URL.
    pub const DEFAULT_BASE_URL: &'static str = "https://api.anthropic.com/v1/";

    /// Default Anthropic API version.
    pub const DEFAULT_VERSION: &'static str = "2023-06-01";

    /// Creates a new Anthropic configuration with default settings.
    ///
    /// Uses `DEFAULT_BASE_URL` and `DEFAULT_VERSION` for the base URL and version.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: Self::DEFAULT_BASE_URL.to_string(),
            anthropic_version: Self::DEFAULT_VERSION.to_string(),
        }
    }

    /// Creates an AnthropicConfig from ChatCompletionSettings.
    ///
    /// # Errors
    ///
    /// Returns `AnthropicConfigError::WrongSource` if the settings source is not Anthropic.
    /// Returns `AnthropicConfigError::MissingApiKey` if the settings don't contain an API key.
    pub fn from_settings(settings: &ChatCompletionSettings) -> Result<Self, AnthropicConfigError> {
        if settings.source != ChatCompletionSource::Anthropic {
            return Err(AnthropicConfigError::WrongSource);
        }

        let api_key = settings.api_key.clone().ok_or(AnthropicConfigError::MissingApiKey)?;

        let base_url = settings
            .base_url
            .clone()
            .or_else(|| settings.source.base_url().map(String::from))
            .unwrap_or_else(|| Self::DEFAULT_BASE_URL.to_string());

        let anthropic_version =
            settings.api_version.clone().unwrap_or_else(|| Self::DEFAULT_VERSION.to_string());

        Ok(Self { api_key, base_url, anthropic_version })
    }

    /// Sets a custom base URL for the Anthropic API.
    #[must_use]
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    /// Sets a custom API version.
    #[must_use]
    pub fn with_version(mut self, version: String) -> Self {
        self.anthropic_version = version;
        self
    }
}

// ============================================================================
// OpenAI-Compatible Configuration
// ============================================================================

/// Configuration for OpenAI-compatible API providers.
///
/// This struct holds credentials and settings for providers that use the OpenAI
/// API format, including OpenAI, Mistral, and vLLM.
///
/// Note: This is the HTTP-independent configuration. The actual HTTP client
/// configuration (using `async_openai`) is handled in the server crate.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenAiCompatibleConfig {
    /// The API key for authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// The organization ID (OpenAI-specific).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    /// The project ID (OpenAI-specific).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// The base URL for the API.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl OpenAiCompatibleConfig {
    /// Creates a new OpenAI-compatible configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a configuration from ChatCompletionSettings.
    ///
    /// # Errors
    ///
    /// Returns an error if the source is Anthropic (use `AnthropicConfig` instead)
    /// or AzureOpenAI (use `AzureOpenAiConfig` instead).
    pub fn from_settings(settings: &ChatCompletionSettings) -> Result<Self, ResponseError> {
        use ChatCompletionSource::*;
        match settings.source {
            Anthropic => Err(ResponseError::from_msg(
                "Use AnthropicConfig for Anthropic source".to_string(),
                Code::BadRequest,
            )),
            AzureOpenAi => Err(ResponseError::from_msg(
                "Use AzureOpenAiConfig for Azure OpenAI source".to_string(),
                Code::BadRequest,
            )),
            OpenAi | Mistral | VLlm => Ok(Self {
                api_key: settings.api_key.clone(),
                org_id: settings.org_id.clone(),
                project_id: settings.project_id.clone(),
                base_url: settings.base_url.clone().or_else(|| settings.source.base_url().map(String::from)),
            }),
        }
    }

    /// Sets the API key.
    #[must_use]
    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    /// Sets the organization ID.
    #[must_use]
    pub fn with_org_id(mut self, org_id: String) -> Self {
        self.org_id = Some(org_id);
        self
    }

    /// Sets the project ID.
    #[must_use]
    pub fn with_project_id(mut self, project_id: String) -> Self {
        self.project_id = Some(project_id);
        self
    }

    /// Sets the base URL.
    #[must_use]
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = Some(base_url);
        self
    }
}

/// Configuration for Azure OpenAI deployments.
///
/// Azure OpenAI uses a different API structure than standard OpenAI, requiring
/// deployment IDs and API versions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AzureOpenAiConfig {
    /// The API key for authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// The base URL for the Azure OpenAI resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// The deployment ID for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    /// The API version to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
}

impl AzureOpenAiConfig {
    /// Creates a new Azure OpenAI configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a configuration from ChatCompletionSettings.
    ///
    /// # Errors
    ///
    /// Returns an error if the source is not AzureOpenAI.
    pub fn from_settings(settings: &ChatCompletionSettings) -> Result<Self, ResponseError> {
        if settings.source != ChatCompletionSource::AzureOpenAi {
            return Err(ResponseError::from_msg(
                "AzureOpenAiConfig requires AzureOpenAi source".to_string(),
                Code::BadRequest,
            ));
        }

        Ok(Self {
            api_key: settings.api_key.clone(),
            base_url: settings.base_url.clone(),
            deployment_id: settings.deployment_id.clone(),
            api_version: settings.api_version.clone(),
        })
    }

    /// Sets the API key.
    #[must_use]
    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    /// Sets the base URL.
    #[must_use]
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = Some(base_url);
        self
    }

    /// Sets the deployment ID.
    #[must_use]
    pub fn with_deployment_id(mut self, deployment_id: String) -> Self {
        self.deployment_id = Some(deployment_id);
        self
    }

    /// Sets the API version.
    #[must_use]
    pub fn with_api_version(mut self, api_version: String) -> Self {
        self.api_version = Some(api_version);
        self
    }
}

// ============================================================================
// Unified Chat Provider Configuration
// ============================================================================

/// Unified configuration for all supported chat providers.
///
/// This enum provides a type-safe way to configure different chat API providers,
/// each with their own specific settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatProviderConfig {
    /// OpenAI API configuration.
    OpenAi(OpenAiCompatibleConfig),
    /// Mistral AI API configuration.
    Mistral(OpenAiCompatibleConfig),
    /// vLLM (OpenAI-compatible) API configuration.
    VLlm(OpenAiCompatibleConfig),
    /// Azure OpenAI API configuration.
    AzureOpenAi(AzureOpenAiConfig),
    /// Anthropic Claude API configuration.
    Anthropic(AnthropicConfig),
}

impl ChatProviderConfig {
    /// Creates a provider configuration from ChatCompletionSettings.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid for the specified source.
    pub fn from_settings(settings: &ChatCompletionSettings) -> Result<Self, ResponseError> {
        use ChatCompletionSource::*;
        match settings.source {
            OpenAi => Ok(Self::OpenAi(OpenAiCompatibleConfig::from_settings(settings)?)),
            Mistral => Ok(Self::Mistral(OpenAiCompatibleConfig::from_settings(settings)?)),
            VLlm => Ok(Self::VLlm(OpenAiCompatibleConfig::from_settings(settings)?)),
            AzureOpenAi => Ok(Self::AzureOpenAi(AzureOpenAiConfig::from_settings(settings)?)),
            Anthropic => {
                let config = AnthropicConfig::from_settings(settings).map_err(|e| {
                    ResponseError::from_msg(e.to_string(), Code::BadRequest)
                })?;
                Ok(Self::Anthropic(config))
            }
        }
    }

    /// Returns the source type for this configuration.
    pub fn source(&self) -> ChatCompletionSource {
        match self {
            Self::OpenAi(_) => ChatCompletionSource::OpenAi,
            Self::Mistral(_) => ChatCompletionSource::Mistral,
            Self::VLlm(_) => ChatCompletionSource::VLlm,
            Self::AzureOpenAi(_) => ChatCompletionSource::AzureOpenAi,
            Self::Anthropic(_) => ChatCompletionSource::Anthropic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_config_new() {
        let config = AnthropicConfig::new("test-key".to_string());
        assert_eq!(config.api_key, "test-key");
        assert_eq!(config.base_url, AnthropicConfig::DEFAULT_BASE_URL);
        assert_eq!(config.anthropic_version, AnthropicConfig::DEFAULT_VERSION);
    }

    #[test]
    fn test_anthropic_config_with_custom_url() {
        let config = AnthropicConfig::new("test-key".to_string())
            .with_base_url("https://custom.api.com/".to_string());
        assert_eq!(config.base_url, "https://custom.api.com/");
    }

    #[test]
    fn test_anthropic_config_from_settings() {
        let settings = ChatCompletionSettings {
            source: ChatCompletionSource::Anthropic,
            api_key: Some("sk-ant-test".to_string()),
            base_url: Some("https://custom.anthropic.com/".to_string()),
            api_version: Some("2024-01-01".to_string()),
            ..Default::default()
        };

        let config = AnthropicConfig::from_settings(&settings).unwrap();
        assert_eq!(config.api_key, "sk-ant-test");
        assert_eq!(config.base_url, "https://custom.anthropic.com/");
        assert_eq!(config.anthropic_version, "2024-01-01");
    }

    #[test]
    fn test_anthropic_config_wrong_source() {
        let settings = ChatCompletionSettings {
            source: ChatCompletionSource::OpenAi,
            api_key: Some("sk-test".to_string()),
            ..Default::default()
        };

        let result = AnthropicConfig::from_settings(&settings);
        assert!(matches!(result, Err(AnthropicConfigError::WrongSource)));
    }

    #[test]
    fn test_anthropic_config_missing_key() {
        let settings = ChatCompletionSettings {
            source: ChatCompletionSource::Anthropic,
            api_key: None,
            ..Default::default()
        };

        let result = AnthropicConfig::from_settings(&settings);
        assert!(matches!(result, Err(AnthropicConfigError::MissingApiKey)));
    }

    #[test]
    fn test_openai_compatible_config_from_settings() {
        let settings = ChatCompletionSettings {
            source: ChatCompletionSource::OpenAi,
            api_key: Some("sk-openai".to_string()),
            org_id: Some("org-123".to_string()),
            ..Default::default()
        };

        let config = OpenAiCompatibleConfig::from_settings(&settings).unwrap();
        assert_eq!(config.api_key, Some("sk-openai".to_string()));
        assert_eq!(config.org_id, Some("org-123".to_string()));
    }

    #[test]
    fn test_azure_config_from_settings() {
        let settings = ChatCompletionSettings {
            source: ChatCompletionSource::AzureOpenAi,
            api_key: Some("azure-key".to_string()),
            base_url: Some("https://myresource.openai.azure.com".to_string()),
            deployment_id: Some("gpt-4".to_string()),
            api_version: Some("2024-02-15-preview".to_string()),
            ..Default::default()
        };

        let config = AzureOpenAiConfig::from_settings(&settings).unwrap();
        assert_eq!(config.api_key, Some("azure-key".to_string()));
        assert_eq!(config.deployment_id, Some("gpt-4".to_string()));
    }

    #[test]
    fn test_chat_provider_config_source() {
        let anthropic = ChatProviderConfig::Anthropic(AnthropicConfig::new("key".to_string()));
        assert_eq!(anthropic.source(), ChatCompletionSource::Anthropic);

        let openai = ChatProviderConfig::OpenAi(OpenAiCompatibleConfig::new());
        assert_eq!(openai.source(), ChatCompletionSource::OpenAi);
    }
}
