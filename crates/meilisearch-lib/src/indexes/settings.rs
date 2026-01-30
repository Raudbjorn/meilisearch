//! Settings operations for meilisearch-lib.
//!
//! This module provides settings management functionality including:
//! - Updating all index settings
//! - Retrieving current settings
//! - Managing embedder configurations for semantic search
//!
//! Settings updates are asynchronous and return a `TaskView` representing
//! the enqueued task. Use the task API to monitor completion.

// Re-export settings types for convenience
pub use meilisearch_types::milli::update::Setting;
pub use meilisearch_types::settings::{
    settings, Checked, SecretPolicy, SettingEmbeddingSettings, Settings, Unchecked,
};

use std::collections::BTreeMap;

use meilisearch_types::tasks::KindWithContent;

use crate::client::MeilisearchLib;
use crate::error::Error;
use crate::tasks::TaskView;

impl MeilisearchLib {
    // =========================================================================
    // Settings Operations
    // =========================================================================

    /// Update the settings of an index.
    ///
    /// This registers a `SettingsUpdate` task with the scheduler. The settings
    /// are not immediately applied; you must wait for the task to complete.
    ///
    /// Passing `Setting::Reset` for any field will reset it to its default value.
    /// Fields set to `Setting::NotSet` will remain unchanged.
    ///
    /// If the index does not exist, it will be created.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    /// * `settings` - The settings to apply (unchecked, will be validated)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index UID is invalid
    /// - The settings are invalid
    /// - There's a database error registering the task
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use meilisearch_lib::Settings;
    /// use meilisearch_types::milli::update::Setting;
    /// use std::collections::BTreeSet;
    ///
    /// let mut settings = Settings::default();
    /// settings.searchable_attributes = Setting::Set(vec!["title".to_string(), "body".to_string()]).into();
    /// settings.filterable_attributes = Setting::Set(vec!["genre".into()]);
    ///
    /// let task = meili.update_settings("movies", settings)?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    /// ```
    pub fn update_settings(
        &self,
        uid: impl Into<String>,
        settings: Settings<Unchecked>,
    ) -> Result<TaskView, Error> {
        let uid = uid.into();

        // Validate the index UID
        if uid.is_empty() {
            return Err(Error::InvalidIndexUid(uid));
        }

        // Validate the settings
        let validated_settings =
            settings.validate().map_err(|e| Error::InvalidSettings(e.to_string()))?;

        let kind = KindWithContent::SettingsUpdate {
            index_uid: uid,
            new_settings: Box::new(validated_settings),
            is_deletion: false,
            allow_index_creation: true,
        };

        let task = self.scheduler().register(kind, None, false)?;
        Ok(TaskView::from(task))
    }

    /// Reset all settings of an index to their default values.
    ///
    /// This is equivalent to calling `update_settings` with all fields set to
    /// `Setting::Reset`.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index doesn't exist (`IndexNotFound`)
    /// - There's a database error registering the task
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let task = meili.reset_settings("movies")?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    /// ```
    pub fn reset_settings(&self, uid: impl Into<String>) -> Result<TaskView, Error> {
        let uid = uid.into();

        // Validate the index UID
        if uid.is_empty() {
            return Err(Error::InvalidIndexUid(uid));
        }

        let cleared_settings = Settings::cleared().into_unchecked();

        let kind = KindWithContent::SettingsUpdate {
            index_uid: uid,
            new_settings: Box::new(cleared_settings),
            is_deletion: true,
            allow_index_creation: false,
        };

        let task = self.scheduler().register(kind, None, false)?;
        Ok(TaskView::from(task))
    }

    /// Get the current settings of an index.
    ///
    /// Returns the complete settings configuration for the index, including
    /// searchable attributes, filterable attributes, ranking rules, etc.
    ///
    /// Note: API keys and other secrets in embedder configurations are
    /// automatically hidden in the response.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index doesn't exist (`IndexNotFound`)
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let settings = meili.get_settings("movies")?;
    /// println!("Searchable attributes: {:?}", settings.searchable_attributes);
    /// println!("Ranking rules: {:?}", settings.ranking_rules);
    /// ```
    pub fn get_settings(&self, uid: impl AsRef<str>) -> Result<Settings<Checked>, Error> {
        let uid = uid.as_ref();
        let index = self.scheduler().index(uid)?;
        let rtxn = index.read_txn()?;

        let current_settings = settings(&index, &rtxn, SecretPolicy::HideSecrets)?;
        Ok(current_settings)
    }

    /// Get the embedder configurations for an index.
    ///
    /// Returns the embedder settings as a JSON value, or `None` if no embedders
    /// are configured. This is useful for checking which embedders are set up
    /// for semantic search.
    ///
    /// Note: API keys are automatically hidden in the response.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index doesn't exist (`IndexNotFound`)
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Some(embedders) = meili.get_embedders("movies")? {
    ///     println!("Embedder config: {}", serde_json::to_string_pretty(&embedders)?);
    /// } else {
    ///     println!("No embedders configured");
    /// }
    /// ```
    pub fn get_embedders(&self, uid: impl AsRef<str>) -> Result<Option<serde_json::Value>, Error> {
        let settings = self.get_settings(uid)?;

        match settings.embedders {
            Setting::Set(embedders) => {
                let value = serde_json::to_value(embedders).map_err(|e| {
                    Error::Internal(format!("failed to serialize embedders: {}", e))
                })?;
                Ok(Some(value))
            }
            Setting::Reset | Setting::NotSet => Ok(None),
        }
    }

    /// Update the embedder configurations for an index.
    ///
    /// This is a convenience method that updates only the embedders setting
    /// while leaving all other settings unchanged.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    /// * `embedders` - The embedder configurations as a JSON value
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index UID is invalid
    /// - The embedder configuration is invalid
    /// - There's a database error registering the task
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use serde_json::json;
    ///
    /// let embedders = json!({
    ///     "default": {
    ///         "source": "openAi",
    ///         "apiKey": "sk-...",
    ///         "model": "text-embedding-3-small",
    ///         "documentTemplate": "A movie titled '{{doc.title}}' described as: {{doc.overview}}"
    ///     }
    /// });
    ///
    /// let task = meili.update_embedders("movies", embedders)?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    /// ```
    pub fn update_embedders(
        &self,
        uid: impl Into<String>,
        embedders: serde_json::Value,
    ) -> Result<TaskView, Error> {
        // Parse the embedders JSON into the expected type
        let embedders_map: BTreeMap<String, SettingEmbeddingSettings> =
            serde_json::from_value(embedders)
                .map_err(|e| Error::InvalidSettings(format!("invalid embedders config: {}", e)))?;

        // Build settings with only the embedders field set
        let settings = Settings { embedders: Setting::Set(embedders_map), ..Default::default() };

        self.update_settings(uid, settings)
    }

    /// Reset the embedder configurations for an index.
    ///
    /// This removes all embedder configurations, disabling semantic search
    /// for the index.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index doesn't exist (`IndexNotFound`)
    /// - There's a database error registering the task
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let task = meili.reset_embedders("movies")?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    /// ```
    pub fn reset_embedders(&self, uid: impl Into<String>) -> Result<TaskView, Error> {
        let settings = Settings { embedders: Setting::Reset, ..Default::default() };

        self.update_settings(uid, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_default() {
        let settings: Settings<Unchecked> = Settings::default();
        // All fields should be NotSet by default
        assert!(matches!(settings.searchable_attributes.as_ref(), Setting::NotSet));
        assert!(matches!(settings.displayed_attributes.as_ref(), Setting::NotSet));
        assert!(matches!(settings.filterable_attributes, Setting::NotSet));
        assert!(matches!(settings.sortable_attributes, Setting::NotSet));
        assert!(matches!(settings.ranking_rules, Setting::NotSet));
        assert!(matches!(settings.embedders, Setting::NotSet));
    }

    #[test]
    fn test_settings_cleared() {
        let settings = Settings::<Checked>::cleared();
        // All fields should be Reset
        assert!(matches!(settings.searchable_attributes.as_ref(), Setting::Reset));
        assert!(matches!(settings.displayed_attributes.as_ref(), Setting::Reset));
        assert!(matches!(settings.filterable_attributes, Setting::Reset));
        assert!(matches!(settings.sortable_attributes, Setting::Reset));
        assert!(matches!(settings.ranking_rules, Setting::Reset));
        assert!(matches!(settings.embedders, Setting::Reset));
    }
}
