use super::*;

impl ProviderService {
    #[allow(dead_code)]
    pub(super) fn parse_common_gemini_config_snippet(snippet: &str) -> Result<Value, AppError> {
        let value: Value = serde_json::from_str(snippet).map_err(|e| {
            AppError::localized(
                "common_config.gemini.invalid_json",
                format!("Gemini 通用配置片段不是有效的 JSON：{e}"),
                format!("Gemini common config snippet is not valid JSON: {e}"),
            )
        })?;
        if !value.is_object() {
            return Err(AppError::localized(
                "common_config.gemini.not_object",
                "Gemini 通用配置片段必须是 JSON 对象",
                "Gemini common config snippet must be a JSON object",
            ));
        }
        Ok(value)
    }

    pub(super) fn prepare_switch_gemini(
        config: &mut MultiAppConfig,
        provider_id: &str,
        _effective_current_provider: Option<&str>,
    ) -> Result<Provider, AppError> {
        let provider = config
            .get_manager(&AppType::Gemini)
            .ok_or_else(|| Self::app_not_found(&AppType::Gemini))?
            .providers
            .get(provider_id)
            .cloned()
            .ok_or_else(|| {
                AppError::localized(
                    "provider.not_found",
                    format!("供应商不存在: {provider_id}"),
                    format!("Provider not found: {provider_id}"),
                )
            })?;

        if let Some(manager) = config.get_manager_mut(&AppType::Gemini) {
            manager.current = provider_id.to_string();
        }

        Ok(provider)
    }

    #[allow(dead_code)]
    pub(super) fn strip_common_gemini_config_from_provider(
        provider: &mut Provider,
        common_config_snippet: Option<&str>,
    ) -> Result<(), AppError> {
        common_config::normalize_provider_common_config_for_storage(
            &AppType::Gemini,
            provider,
            common_config_snippet,
        )
    }

    fn migrate_common_gemini_config_from_provider(
        provider: &mut Provider,
        common_config_snippet: Option<&str>,
    ) -> Result<(), AppError> {
        common_config::migrate_provider_subset_usage_for_storage(
            &AppType::Gemini,
            provider,
            common_config_snippet,
        )
    }

    pub(super) fn migrate_gemini_common_config_snippet(
        config: &mut MultiAppConfig,
        strict_current_provider_id: Option<&str>,
        old_snippet: &str,
    ) -> Result<(), AppError> {
        let old_snippet = old_snippet.trim();
        if old_snippet.is_empty() {
            return Ok(());
        }

        let Some(current_provider_id) = strict_current_provider_id.and_then(|provider_id| {
            config.get_manager(&AppType::Gemini).and_then(|manager| {
                manager
                    .providers
                    .contains_key(provider_id)
                    .then(|| provider_id.to_string())
            })
        }) else {
            let Some(manager) = config.get_manager_mut(&AppType::Gemini) else {
                return Ok(());
            };

            for provider in manager.providers.values_mut() {
                Self::migrate_common_gemini_config_from_provider(provider, Some(old_snippet))?;
            }

            return Ok(());
        };

        let Some(manager) = config.get_manager_mut(&AppType::Gemini) else {
            return Ok(());
        };

        if let Some(current_provider) = manager.providers.get_mut(&current_provider_id) {
            Self::migrate_common_gemini_config_from_provider(current_provider, Some(old_snippet))?;
        }

        for (provider_id, provider) in manager.providers.iter_mut() {
            if provider_id == &current_provider_id {
                continue;
            }

            if let Err(err) =
                Self::migrate_common_gemini_config_from_provider(provider, Some(old_snippet))
            {
                log::warn!(
                    "skip migrating Gemini non-current provider snapshot '{provider_id}' from stored common config snippet: {err}"
                );
            }
        }

        Ok(())
    }

    pub(crate) fn write_gemini_live_force(
        provider: &Provider,
        common_config_snippet: Option<&str>,
    ) -> Result<(), AppError> {
        let prepared =
            Self::prepare_gemini_live_write(provider, common_config_snippet, None, true)?;
        Self::apply_gemini_live_write(&prepared)
    }

    pub(super) fn prepare_gemini_live_write(
        provider: &Provider,
        _common_config_snippet: Option<&str>,
        _previous_common_config_snippet: Option<&str>,
        force_sync: bool,
    ) -> Result<PreparedLiveWrite, AppError> {
        if !force_sync && !crate::sync_policy::should_sync_live(&AppType::Gemini) {
            return Ok(PreparedLiveWrite::Noop);
        }
        let official = Self::is_official_gemini(provider);
        crate::gemini_config::validate_gemini_settings(&provider.settings_config)?;
        if !official {
            crate::gemini_config::validate_gemini_settings_strict(&provider.settings_config)?;
        }
        let projection =
            crate::live::project::gemini::GeminiProjection::of(&provider.settings_config, official);
        Ok(PreparedLiveWrite::Gemini {
            env: projection.env_patch(),
            settings: projection.settings_patch(),
        })
    }

    pub(super) fn apply_gemini_live_write(prepared: &PreparedLiveWrite) -> Result<(), AppError> {
        key_field_live::apply(prepared, None)
    }
}
