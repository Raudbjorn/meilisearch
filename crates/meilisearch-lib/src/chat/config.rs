//! Chat configuration types.

// TODO: Implement ChatConfig and related types
// See Design.md section 4.1 for the full implementation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// LLM provider type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ChatSource {
    /// OpenAI API.
    OpenAi,
    /// Anthropic Claude API.
    Anthropic,
    /// Azure OpenAI Service.
    AzureOpenAi,
    /// Mistral AI API.
    Mistral,
    /// vLLM server.
    VLlm,
}

/// Chat configuration (replaces workspace LMDB persistence).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConfig {
    /// LLM provider.
    pub source: ChatSource,
    /// API key for the provider.
    pub api_key: String,
    /// Custom base URL (required for Azure, vLLM).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Model identifier.
    pub model: String,
    /// Organization ID (OpenAI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    /// Project ID (OpenAI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// API version (Azure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    /// Deployment ID (Azure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    /// Prompt configuration.
    #[serde(default)]
    pub prompts: ChatPrompts,
    /// Per-index chat configuration.
    #[serde(default)]
    pub index_configs: HashMap<String, ChatIndexConfig>,
}

/// Prompt templates for chat.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatPrompts {
    /// System prompt for the LLM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Description of search function capabilities.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_description: Option<String>,
    /// Instructions for query parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_q_param: Option<String>,
    /// Instructions for filter parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_filter_param: Option<String>,
    /// Instructions for index selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_index_uid_param: Option<String>,
}

/// Per-index chat configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatIndexConfig {
    /// Human-readable description of index contents.
    pub description: String,
    /// Liquid template for rendering documents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Max bytes per document (default: 400).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
    /// Search parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_params: Option<ChatSearchParams>,
}

/// Search parameters for chat context retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSearchParams {
    /// Max documents to retrieve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Sort criteria.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<String>>,
    /// Matching strategy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_strategy: Option<String>,
    /// Semantic ratio for hybrid search (0.0-1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_ratio: Option<f32>,
    /// Which embedder to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedder: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test the complete chat configuration lifecycle.
    ///
    /// This test verifies:
    /// 1. ChatConfig can be created with all fields
    /// 2. Serialization produces correct camelCase JSON
    /// 3. Deserialization reconstructs the config correctly
    /// 4. Optional fields are skipped when None
    /// 5. All ChatSource variants serialize correctly
    #[test]
    fn test_chat_config_lifecycle() {
        // Create a full config
        let mut index_configs = HashMap::new();
        index_configs.insert(
            "movies".to_string(),
            ChatIndexConfig {
                description: "Movie database".to_string(),
                template: Some("Title: {{ title }}".to_string()),
                max_bytes: Some(500),
                search_params: Some(ChatSearchParams {
                    limit: Some(10),
                    sort: Some(vec!["rating:desc".to_string()]),
                    matching_strategy: Some("all".to_string()),
                    semantic_ratio: Some(0.7),
                    embedder: Some("default".to_string()),
                }),
            },
        );

        let config = ChatConfig {
            source: ChatSource::OpenAi,
            api_key: "sk-test-key".to_string(),
            base_url: Some("https://api.openai.com/v1".to_string()),
            model: "gpt-4".to_string(),
            org_id: Some("org-123".to_string()),
            project_id: Some("proj-456".to_string()),
            api_version: None,
            deployment_id: None,
            prompts: ChatPrompts {
                system: Some("You are a helpful assistant.".to_string()),
                search_description: Some("Search the database".to_string()),
                search_q_param: None,
                search_filter_param: None,
                search_index_uid_param: None,
            },
            index_configs,
        };

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&config).expect("serialization failed");

        // Verify camelCase field names
        assert!(json.contains("\"source\""), "should have source field");
        assert!(json.contains("\"apiKey\""), "should use camelCase for api_key");
        assert!(json.contains("\"baseUrl\""), "should use camelCase for base_url");
        assert!(json.contains("\"orgId\""), "should use camelCase for org_id");
        assert!(json.contains("\"projectId\""), "should use camelCase for project_id");
        assert!(json.contains("\"indexConfigs\""), "should use camelCase for index_configs");

        // Verify optional fields with None are skipped
        assert!(!json.contains("\"apiVersion\""), "None api_version should be skipped");
        assert!(!json.contains("\"deploymentId\""), "None deployment_id should be skipped");

        // Round-trip deserialization
        let deserialized: ChatConfig =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.source, ChatSource::OpenAi);
        assert_eq!(deserialized.api_key, "sk-test-key");
        assert_eq!(deserialized.model, "gpt-4");
        assert_eq!(deserialized.org_id, Some("org-123".to_string()));
        assert!(deserialized.api_version.is_none());

        // Verify nested config
        let movie_config = deserialized.index_configs.get("movies").expect("movie config missing");
        assert_eq!(movie_config.description, "Movie database");
        assert_eq!(movie_config.max_bytes, Some(500));

        let search_params = movie_config.search_params.as_ref().expect("search params missing");
        assert_eq!(search_params.limit, Some(10));
        assert_eq!(search_params.semantic_ratio, Some(0.7));
    }

    /// Test all ChatSource variants serialize correctly.
    #[test]
    fn test_chat_source_variants() {
        let sources = [
            (ChatSource::OpenAi, "\"openAi\""),
            (ChatSource::Anthropic, "\"anthropic\""),
            (ChatSource::AzureOpenAi, "\"azureOpenAi\""),
            (ChatSource::Mistral, "\"mistral\""),
            (ChatSource::VLlm, "\"vLlm\""),
        ];

        for (source, expected) in sources {
            let json = serde_json::to_string(&source).expect("serialization failed");
            assert_eq!(json, expected, "ChatSource::{:?} should serialize to {}", source, expected);

            // Round-trip
            let deserialized: ChatSource =
                serde_json::from_str(&json).expect("deserialization failed");
            assert_eq!(deserialized, source);
        }
    }

    /// Test ChatConfig serialization for file persistence (DD-004).
    #[test]
    fn test_chat_config_serialization() {
        // Minimal config for Anthropic
        let config = ChatConfig {
            source: ChatSource::Anthropic,
            api_key: "sk-ant-api-key".to_string(),
            base_url: None,
            model: "claude-3-sonnet-20240229".to_string(),
            org_id: None,
            project_id: None,
            api_version: None,
            deployment_id: None,
            prompts: ChatPrompts::default(),
            index_configs: HashMap::new(),
        };

        // Serialize and deserialize
        let json = serde_json::to_string(&config).expect("serialization failed");
        let restored: ChatConfig = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(restored.source, ChatSource::Anthropic);
        assert_eq!(restored.model, "claude-3-sonnet-20240229");
        assert!(restored.index_configs.is_empty());

        // Test that it can be saved to a file and loaded back
        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_chat_config.json");

        std::fs::write(&config_path, &json).expect("failed to write config file");
        let loaded_json = std::fs::read_to_string(&config_path).expect("failed to read config file");
        let loaded: ChatConfig = serde_json::from_str(&loaded_json).expect("failed to parse loaded config");

        assert_eq!(loaded.source, config.source);
        assert_eq!(loaded.api_key, config.api_key);
        assert_eq!(loaded.model, config.model);

        // Cleanup
        let _ = std::fs::remove_file(&config_path);
    }

    /// Test ChatPrompts defaults.
    #[test]
    fn test_chat_prompts_default() {
        let prompts = ChatPrompts::default();

        assert!(prompts.system.is_none());
        assert!(prompts.search_description.is_none());
        assert!(prompts.search_q_param.is_none());
        assert!(prompts.search_filter_param.is_none());
        assert!(prompts.search_index_uid_param.is_none());
    }
}
