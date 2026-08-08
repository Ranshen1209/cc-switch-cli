use crate::claude_desktop_config::ONE_M_CONTEXT_MARKER;
use crate::provider::Provider;
use serde_json::Value;

pub struct ModelMapping {
    pub haiku_model: Option<String>,
    pub sonnet_model: Option<String>,
    pub opus_model: Option<String>,
    pub fable_model: Option<String>,
    pub subagent_model: Option<String>,
    pub default_model: Option<String>,
}

impl ModelMapping {
    pub fn from_provider(provider: &Provider) -> Self {
        let env = provider.settings_config.get("env");

        Self {
            haiku_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_HAIKU_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            sonnet_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_SONNET_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            opus_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_OPUS_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            fable_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_FABLE_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            subagent_model: env
                .and_then(|value| value.get("CLAUDE_CODE_SUBAGENT_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            default_model: env
                .and_then(|value| value.get("ANTHROPIC_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
        }
    }

    pub fn has_mapping(&self) -> bool {
        self.haiku_model.is_some()
            || self.sonnet_model.is_some()
            || self.opus_model.is_some()
            || self.fable_model.is_some()
            || self.subagent_model.is_some()
            || self.default_model.is_some()
    }

    pub fn map_model(&self, original_model: &str) -> String {
        let model_lower = original_model.to_lowercase();

        if model_lower.contains("fable") {
            if let Some(model) = &self.fable_model {
                return model.clone();
            }
            // Providers without a dedicated Fable tier should retain the closest tier mapping.
            if let Some(model) = &self.opus_model {
                return model.clone();
            }
        }
        if model_lower.contains("haiku") {
            if let Some(model) = &self.haiku_model {
                return model.clone();
            }
        }
        if model_lower.contains("opus") {
            if let Some(model) = &self.opus_model {
                return model.clone();
            }
        }
        if model_lower.contains("sonnet") {
            if let Some(model) = &self.sonnet_model {
                return model.clone();
            }
        }

        // subagent 模型保护：若请求的模型（忽略 [1M] 后缀）与 CLAUDE_CODE_SUBAGENT_MODEL
        // 一致，说明这是子 agent 使用自己的专属模型，不应被 default_model 覆盖，直接保持原样。
        if let Some(ref m) = self.subagent_model {
            if strip_one_m_suffix_for_upstream(original_model) == strip_one_m_suffix_for_upstream(m)
            {
                return original_model.to_string();
            }
        }

        if let Some(model) = &self.default_model {
            return model.clone();
        }

        original_model.to_string()
    }
}

pub fn apply_model_mapping(
    mut body: Value,
    provider: &Provider,
) -> (Value, Option<String>, Option<String>) {
    let mapping = ModelMapping::from_provider(provider);

    if !mapping.has_mapping() {
        let original = body.get("model").and_then(Value::as_str).map(String::from);
        return (body, original, None);
    }

    let original_model = body.get("model").and_then(Value::as_str).map(String::from);

    if let Some(original) = &original_model {
        let mapped = mapping.map_model(original);

        if mapped != *original {
            body["model"] = serde_json::json!(mapped);
            return (body, Some(original.clone()), Some(mapped));
        }
    }

    (body, original_model, None)
}

pub fn strip_one_m_suffix_for_upstream(model: &str) -> &str {
    let trimmed = model.trim_end();
    let marker = ONE_M_CONTEXT_MARKER.as_bytes();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= marker.len()
        && bytes[bytes.len() - marker.len()..].eq_ignore_ascii_case(marker)
    {
        return trimmed[..trimmed.len() - marker.len()].trim_end();
    }
    model
}

pub fn strip_one_m_suffix_for_upstream_from_body(mut body: Value) -> Value {
    let Some(model) = body.get("model").and_then(Value::as_str) else {
        return body;
    };

    let stripped = strip_one_m_suffix_for_upstream(model);
    if stripped != model {
        body["model"] = serde_json::json!(stripped);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider_with_mapping(mapped_model: &str) -> Provider {
        Provider {
            id: "test".to_string(),
            name: "Test".to_string(),
            settings_config: json!({
                "env": {
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": mapped_model
                }
            }),
            website_url: None,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    fn provider_without_mapping() -> Provider {
        Provider {
            id: "test".to_string(),
            name: "Test".to_string(),
            settings_config: json!({}),
            website_url: None,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    #[test]
    fn thinking_does_not_use_legacy_reasoning_model_mapping() {
        let mut provider = provider_with_mapping("sonnet-mapped");
        provider.settings_config["env"]["ANTHROPIC_REASONING_MODEL"] = json!("reasoning-mapped");
        let body = json!({
            "model": "claude-sonnet-4-6",
            "thinking": {"type": "enabled"}
        });

        let (result, _, mapped) = apply_model_mapping(body, &provider);

        assert_eq!(result["model"], "sonnet-mapped");
        assert_eq!(mapped, Some("sonnet-mapped".to_string()));
    }

    #[test]
    fn fable_uses_dedicated_mapping_when_configured() {
        let mut provider = provider_with_mapping("sonnet-mapped");
        provider.settings_config["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"] = json!("fable-mapped");

        let (result, _, mapped) =
            apply_model_mapping(json!({"model": "claude-fable-5[1m]"}), &provider);

        assert_eq!(result["model"], "fable-mapped");
        assert_eq!(mapped, Some("fable-mapped".to_string()));
    }

    #[test]
    fn fable_falls_back_to_opus_mapping_when_unset() {
        let mut provider = provider_with_mapping("sonnet-mapped");
        provider.settings_config["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"] = json!("opus-mapped");

        let (result, _, mapped) =
            apply_model_mapping(json!({"model": "claude-fable-5"}), &provider);

        assert_eq!(result["model"], "opus-mapped");
        assert_eq!(mapped, Some("opus-mapped".to_string()));
    }

    #[test]
    fn fable_falls_back_to_default_mapping_without_opus() {
        let mut provider = provider_with_mapping("sonnet-mapped");
        provider.settings_config["env"]["ANTHROPIC_MODEL"] = json!("default-mapped");

        let (result, _, mapped) =
            apply_model_mapping(json!({"model": "claude-fable-5"}), &provider);

        assert_eq!(result["model"], "default-mapped");
        assert_eq!(mapped, Some("default-mapped".to_string()));
    }

    #[test]
    fn strips_one_m_suffix_before_upstream() {
        let body = json!({"model": "deepseek-v4-pro[1M]"});
        let result = strip_one_m_suffix_for_upstream_from_body(body);
        assert_eq!(result["model"], "deepseek-v4-pro");
    }

    #[test]
    fn strips_one_m_suffix_after_mapping() {
        let provider = provider_with_mapping("deepseek-v4-pro [1M]");
        let body = json!({"model": "claude-sonnet-4-6"});

        let (mapped, _, _) = apply_model_mapping(body, &provider);
        let result = strip_one_m_suffix_for_upstream_from_body(mapped);

        assert_eq!(result["model"], "deepseek-v4-pro");
    }

    #[test]
    fn keeps_model_without_one_m_suffix() {
        let body = json!({"model": "deepseek-v4-pro"});
        let result = strip_one_m_suffix_for_upstream_from_body(body);
        assert_eq!(result["model"], "deepseek-v4-pro");
    }

    #[test]
    fn no_mapping_configured_passes_model_through() {
        let provider = provider_without_mapping();
        let body = json!({"model": "claude-sonnet-4-5"});
        let (result, original, mapped) = apply_model_mapping(body, &provider);
        assert_eq!(result["model"], "claude-sonnet-4-5");
        assert_eq!(original, Some("claude-sonnet-4-5".to_string()));
        assert!(mapped.is_none());
    }

    #[test]
    fn subagent_model_preserved_before_default_fallback() {
        // CLAUDE_CODE_SUBAGENT_MODEL 配置的模型不应被 ANTHROPIC_MODEL 覆盖；
        // 子 agent 使用自己的专属模型发请求时，proxy 应保持原样转发。
        let mut provider = provider_with_mapping("sonnet-mapped");
        provider.settings_config["env"]["ANTHROPIC_MODEL"] = json!("default-model");
        provider.settings_config["env"]["CLAUDE_CODE_SUBAGENT_MODEL"] = json!("gpt-5.4-mini");

        let body = json!({"model": "gpt-5.4-mini"});
        let (result, original, mapped) = apply_model_mapping(body, &provider);

        assert_eq!(result["model"], "gpt-5.4-mini");
        assert_eq!(original, Some("gpt-5.4-mini".to_string()));
        assert!(mapped.is_none());
    }

    #[test]
    fn subagent_model_preserved_with_one_m_suffix() {
        // 子 agent 附带 [1M] 后缀发请求时同样应保持原样，[1M] 不影响 subagent 模型识别。
        let mut provider = provider_with_mapping("sonnet-mapped");
        provider.settings_config["env"]["ANTHROPIC_MODEL"] = json!("default-model");
        provider.settings_config["env"]["CLAUDE_CODE_SUBAGENT_MODEL"] = json!("gpt-5.4-mini");

        let body = json!({"model": "gpt-5.4-mini[1M]"});
        let (result, original, mapped) = apply_model_mapping(body, &provider);

        assert_eq!(result["model"], "gpt-5.4-mini[1M]");
        assert_eq!(original, Some("gpt-5.4-mini[1M]".to_string()));
        assert!(mapped.is_none());
    }
}
