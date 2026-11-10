use super::*;

impl ProviderService {
    /// 归一化 Claude 模型键：读旧键(ANTHROPIC_SMALL_FAST_MODEL)，写新键(DEFAULT_*), 并删除旧键
    pub(crate) fn normalize_claude_models_in_value(settings: &mut Value) -> bool {
        let mut changed = false;
        let env = match settings.get_mut("env") {
            Some(v) if v.is_object() => v.as_object_mut().unwrap(),
            _ => return changed,
        };

        let model = env
            .get("ANTHROPIC_MODEL")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let small_fast = env
            .get("ANTHROPIC_SMALL_FAST_MODEL")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let current_haiku = env
            .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let current_sonnet = env
            .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let current_opus = env
            .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let target_haiku = current_haiku
            .or_else(|| small_fast.clone())
            .or_else(|| model.clone());
        let target_sonnet = current_sonnet
            .or_else(|| model.clone())
            .or_else(|| small_fast.clone());
        let target_opus = current_opus
            .or_else(|| model.clone())
            .or_else(|| small_fast.clone());

        if env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL").is_none() {
            if let Some(v) = target_haiku {
                env.insert(
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
                    Value::String(v),
                );
                changed = true;
            }
        }
        if env.get("ANTHROPIC_DEFAULT_SONNET_MODEL").is_none() {
            if let Some(v) = target_sonnet {
                env.insert(
                    "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
                    Value::String(v),
                );
                changed = true;
            }
        }
        if env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").is_none() {
            if let Some(v) = target_opus {
                env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(), Value::String(v));
                changed = true;
            }
        }

        if env.remove("ANTHROPIC_SMALL_FAST_MODEL").is_some() {
            changed = true;
        }

        changed
    }

    pub(super) fn normalize_provider_if_claude(app_type: &AppType, provider: &mut Provider) {
        if matches!(app_type, AppType::Claude) {
            let mut v = provider.settings_config.clone();
            if Self::normalize_claude_models_in_value(&mut v) {
                provider.settings_config = v;
            }
        }
    }

    #[allow(dead_code)]
    pub(super) fn strip_common_claude_config_from_provider(
        provider: &mut Provider,
        common_config_snippet: Option<&str>,
    ) -> Result<(), AppError> {
        common_config::normalize_provider_common_config_for_storage(
            &AppType::Claude,
            provider,
            common_config_snippet,
        )
    }

    pub(super) fn prepare_switch_claude(
        config: &mut MultiAppConfig,
        provider_id: &str,
        _effective_current_provider: Option<&str>,
    ) -> Result<Provider, AppError> {
        let provider = config
            .get_manager(&AppType::Claude)
            .ok_or_else(|| Self::app_not_found(&AppType::Claude))?
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

        if let Some(manager) = config.get_manager_mut(&AppType::Claude) {
            manager.current = provider_id.to_string();
        }

        Ok(provider)
    }

    pub(super) fn migrate_claude_common_config_snippet(
        config: &mut MultiAppConfig,
        old_snippet: &str,
    ) -> Result<(), AppError> {
        let old_snippet = old_snippet.trim();
        if old_snippet.is_empty() {
            return Ok(());
        }

        let Some(manager) = config.get_manager_mut(&AppType::Claude) else {
            return Ok(());
        };

        for provider in manager.providers.values_mut() {
            common_config::migrate_provider_subset_usage_for_storage(
                &AppType::Claude,
                provider,
                Some(old_snippet),
            )?;
        }

        Ok(())
    }

    pub(crate) fn write_claude_live_force(
        provider: &Provider,
        common_config_snippet: Option<&str>,
        apply_common_config: bool,
    ) -> Result<(), AppError> {
        let prepared = Self::prepare_claude_live_write(
            provider,
            common_config_snippet,
            None,
            apply_common_config,
            true,
            None,
        )?;
        Self::apply_claude_live_write(&prepared)
    }

    pub(super) fn prepare_claude_live_write(
        provider: &Provider,
        _common_config_snippet: Option<&str>,
        _previous_common_config_snippet: Option<&str>,
        _apply_common_config: bool,
        force_sync: bool,
        previous: Option<&Provider>,
    ) -> Result<PreparedLiveWrite, AppError> {
        if !force_sync && !crate::sync_policy::should_sync_live(&AppType::Claude) {
            return Ok(PreparedLiveWrite::Noop);
        }
        use crate::live::project::claude::{direct_patch, ClaudeProjection};
        let previous = previous.map(|p| ClaudeProjection::of(&p.settings_config));
        Ok(PreparedLiveWrite::Claude {
            patch: direct_patch(
                previous.as_ref(),
                &ClaudeProjection::of(&provider.settings_config),
            ),
        })
    }

    pub(super) fn apply_claude_live_write(prepared: &PreparedLiveWrite) -> Result<(), AppError> {
        key_field_live::apply(prepared, None)
    }
}
