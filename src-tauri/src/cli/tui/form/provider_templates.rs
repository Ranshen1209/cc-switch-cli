use crate::app_config::AppType;
use crate::provider::{ClaudeApiKeyField, CodexChatReasoningConfig};
use serde_json::json;

use super::{
    ClaudeApiFormat, CodexModelCatalogField, CodexWireApi, FormMode, GeminiAuthType,
    ProviderAddFormState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderTemplateId {
    Custom,
    ClaudeOfficial,
    CodexOAuth,
    OpenAiOfficial,
    GoogleOAuth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderTemplateDef {
    id: ProviderTemplateId,
    label: &'static str,
}

static PROVIDER_TEMPLATE_DEFS_CLAUDE: [ProviderTemplateDef; 3] = [
    ProviderTemplateDef {
        id: ProviderTemplateId::Custom,
        label: "Custom",
    },
    ProviderTemplateDef {
        id: ProviderTemplateId::ClaudeOfficial,
        label: "Claude Official",
    },
    ProviderTemplateDef {
        id: ProviderTemplateId::CodexOAuth,
        label: "Codex",
    },
];

static PROVIDER_TEMPLATE_DEFS_CLAUDE_DESKTOP: [ProviderTemplateDef; 1] = [ProviderTemplateDef {
    id: ProviderTemplateId::Custom,
    label: "Custom",
}];

static PROVIDER_TEMPLATE_DEFS_CODEX: [ProviderTemplateDef; 2] = [
    ProviderTemplateDef {
        id: ProviderTemplateId::Custom,
        label: "Custom",
    },
    ProviderTemplateDef {
        id: ProviderTemplateId::OpenAiOfficial,
        label: "OpenAI Official",
    },
];

static PROVIDER_TEMPLATE_DEFS_GEMINI: [ProviderTemplateDef; 2] = [
    ProviderTemplateDef {
        id: ProviderTemplateId::Custom,
        label: "Custom",
    },
    ProviderTemplateDef {
        id: ProviderTemplateId::GoogleOAuth,
        label: "Google OAuth",
    },
];

static PROVIDER_TEMPLATE_DEFS_OPENCODE: [ProviderTemplateDef; 1] = [ProviderTemplateDef {
    id: ProviderTemplateId::Custom,
    label: "Custom",
}];

static PROVIDER_TEMPLATE_DEFS_HERMES: [ProviderTemplateDef; 1] = [ProviderTemplateDef {
    id: ProviderTemplateId::Custom,
    label: "Custom",
}];

static PROVIDER_TEMPLATE_DEFS_OPENCLAW: [ProviderTemplateDef; 1] = [ProviderTemplateDef {
    id: ProviderTemplateId::Custom,
    label: "Custom",
}];

pub(super) fn provider_builtin_template_defs(app_type: &AppType) -> &'static [ProviderTemplateDef] {
    match app_type {
        AppType::Claude => &PROVIDER_TEMPLATE_DEFS_CLAUDE,
        AppType::ClaudeDesktop => &PROVIDER_TEMPLATE_DEFS_CLAUDE_DESKTOP,
        AppType::Codex => &PROVIDER_TEMPLATE_DEFS_CODEX,
        AppType::Gemini => &PROVIDER_TEMPLATE_DEFS_GEMINI,
        AppType::OpenCode => &PROVIDER_TEMPLATE_DEFS_OPENCODE,
        AppType::Hermes => &PROVIDER_TEMPLATE_DEFS_HERMES,
        AppType::OpenClaw => &PROVIDER_TEMPLATE_DEFS_OPENCLAW,
    }
}

impl ProviderAddFormState {
    pub fn template_count(&self) -> usize {
        provider_builtin_template_defs(&self.app_type).len()
    }

    pub fn template_labels(&self) -> Vec<&'static str> {
        provider_builtin_template_defs(&self.app_type)
            .iter()
            .map(|def| def.label)
            .collect()
    }

    pub fn apply_template(&mut self, idx: usize, existing_ids: &[String]) {
        let builtin_defs = provider_builtin_template_defs(&self.app_type);
        let total_templates = builtin_defs.len();
        let idx = idx.min(total_templates.saturating_sub(1));
        self.template_idx = idx;
        self.id_is_manual = false;

        let template_id = builtin_defs
            .get(idx)
            .map(|def| def.id)
            .unwrap_or(ProviderTemplateId::Custom);

        if template_id == ProviderTemplateId::Custom {
            if matches!(self.mode, FormMode::Add) {
                let defaults = Self::new(self.app_type.clone());
                let previous_include_common_config = self.include_common_config;
                let previous_include_common_config_touched = self.include_common_config_touched;
                self.extra = defaults.extra;
                self.id = defaults.id;
                self.id_is_manual = defaults.id_is_manual;
                self.name = defaults.name;
                self.website_url = defaults.website_url;
                self.notes = defaults.notes;
                self.include_common_config = previous_include_common_config;
                self.include_common_config_touched = previous_include_common_config_touched;
                self.json_scroll = defaults.json_scroll;
                self.codex_preview_section = defaults.codex_preview_section;
                self.codex_auth_scroll = defaults.codex_auth_scroll;
                self.codex_config_scroll = defaults.codex_config_scroll;
                self.claude_model_config_touched = defaults.claude_model_config_touched;
                self.claude_api_key = defaults.claude_api_key;
                self.claude_api_key_field = defaults.claude_api_key_field;
                self.claude_base_url = defaults.claude_base_url;
                self.claude_api_format = defaults.claude_api_format;
                self.claude_model = defaults.claude_model;
                self.claude_reasoning_model = defaults.claude_reasoning_model;
                self.claude_haiku_model = defaults.claude_haiku_model;
                self.claude_sonnet_model = defaults.claude_sonnet_model;
                self.claude_opus_model = defaults.claude_opus_model;
                self.claude_hide_attribution = defaults.claude_hide_attribution;
                self.claude_teammates = defaults.claude_teammates;
                self.claude_tool_search = defaults.claude_tool_search;
                self.claude_disable_auto_upgrade = defaults.claude_disable_auto_upgrade;
                self.codex_oauth_account_id = defaults.codex_oauth_account_id;
                self.codex_fast_mode = defaults.codex_fast_mode;
                self.codex_base_url = defaults.codex_base_url;
                self.codex_model = defaults.codex_model;
                self.codex_wire_api = defaults.codex_wire_api;
                self.codex_requires_openai_auth = defaults.codex_requires_openai_auth;
                self.codex_env_key = defaults.codex_env_key;
                self.codex_api_key = defaults.codex_api_key;
                self.codex_chat_reasoning = defaults.codex_chat_reasoning;
                self.codex_model_catalog = defaults.codex_model_catalog;
                self.codex_local_routing_enabled = defaults.codex_local_routing_enabled;
                self.codex_goal_mode = defaults.codex_goal_mode;
                self.codex_remote_compaction = defaults.codex_remote_compaction;
                self.codex_local_routing_field_idx = defaults.codex_local_routing_field_idx;
                self.codex_model_catalog_idx = defaults.codex_model_catalog_idx;
                self.codex_model_catalog_field = defaults.codex_model_catalog_field;
                self.gemini_auth_type = defaults.gemini_auth_type;
                self.gemini_api_key = defaults.gemini_api_key;
                self.gemini_base_url = defaults.gemini_base_url;
                self.gemini_model = defaults.gemini_model;
                self.openclaw_user_agent = defaults.openclaw_user_agent;
                self.openclaw_models = defaults.openclaw_models;
                self.hermes_api_mode = defaults.hermes_api_mode;
                self.hermes_api_key = defaults.hermes_api_key;
                self.hermes_base_url = defaults.hermes_base_url;
                self.hermes_models = defaults.hermes_models;
                self.hermes_rate_limit_delay = defaults.hermes_rate_limit_delay;
                self.opencode_npm_package = defaults.opencode_npm_package;
                self.opencode_api_key = defaults.opencode_api_key;
                self.opencode_base_url = defaults.opencode_base_url;
                self.opencode_model_id = defaults.opencode_model_id;
                self.opencode_model_name = defaults.opencode_model_name;
                self.opencode_model_context_limit = defaults.opencode_model_context_limit;
                self.opencode_model_output_limit = defaults.opencode_model_output_limit;
                self.opencode_model_original_id = defaults.opencode_model_original_id;
            }
            return;
        }

        self.extra = json!({});
        self.notes.set("");
        match template_id {
            ProviderTemplateId::Custom => {}
            ProviderTemplateId::ClaudeOfficial => {
                self.extra = json!({
                    "category": "official",
                });
                self.name.set("Claude Official");
                self.website_url
                    .set("https://www.anthropic.com/claude-code");
                self.claude_api_key.set("");
                self.claude_api_key_field = ClaudeApiKeyField::AuthToken;
                self.claude_base_url.set("");
                self.claude_api_format = ClaudeApiFormat::Anthropic;
                self.claude_model.set("");
                self.claude_reasoning_model.set("");
                self.claude_haiku_model.set("");
                self.claude_sonnet_model.set("");
                self.claude_opus_model.set("");
                self.claude_model_config_touched = false;
                self.codex_oauth_account_id = None;
                self.codex_fast_mode = false;
                self.claude_hide_attribution = false;
                self.claude_hide_attribution_touched = false;
                self.claude_teammates = false;
                self.claude_teammates_touched = false;
                self.claude_tool_search = false;
                self.claude_tool_search_touched = false;
                self.claude_disable_auto_upgrade = false;
                self.claude_disable_auto_upgrade_touched = false;
            }
            ProviderTemplateId::CodexOAuth => {
                self.extra = json!({
                    "meta": {
                        "providerType": "codex_oauth",
                        "authBinding": {
                            "source": "managed_account",
                            "authProvider": "codex_oauth",
                        },
                    }
                });
                self.name.set("Codex");
                self.website_url.set("https://openai.com/chatgpt/pricing");
                self.claude_api_key.set("");
                self.claude_api_key_field = ClaudeApiKeyField::AuthToken;
                self.claude_base_url
                    .set("https://chatgpt.com/backend-api/codex");
                self.claude_api_format = ClaudeApiFormat::OpenAiResponses;
                self.claude_model.set("gpt-5.4");
                self.claude_reasoning_model.set("gpt-5.4");
                self.claude_haiku_model.set("gpt-5.4-mini");
                self.claude_sonnet_model.set("gpt-5.4");
                self.claude_opus_model.set("gpt-5.4");
                self.claude_model_config_touched = true;
                self.codex_oauth_account_id = None;
                self.codex_fast_mode = false;
                self.claude_hide_attribution = true;
                self.claude_hide_attribution_touched = true;
                self.claude_teammates = false;
                self.claude_teammates_touched = false;
                self.claude_tool_search = false;
                self.claude_tool_search_touched = false;
                self.claude_disable_auto_upgrade = false;
                self.claude_disable_auto_upgrade_touched = false;
            }
            ProviderTemplateId::OpenAiOfficial => {
                self.extra = json!({
                    "category": "official",
                    "meta": {
                        "codexOfficial": true,
                    }
                });
                self.name.set("OpenAI Official");
                self.website_url.set("https://chatgpt.com/codex");
                self.codex_api_key.set("");
                self.codex_base_url.set("");
                self.codex_model.set("");
                self.codex_wire_api = CodexWireApi::Responses;
                self.codex_requires_openai_auth = true;
                self.codex_env_key.set("");
                self.reset_codex_local_routing_state();
            }
            ProviderTemplateId::GoogleOAuth => {
                self.extra = json!({
                    "category": "official",
                    "meta": {
                        "partnerPromotionKey": "google-official",
                    }
                });
                self.name.set("Google OAuth");
                self.website_url.set("https://ai.google.dev");
                self.gemini_auth_type = GeminiAuthType::OAuth;
            }
        };

        // A preset with a model catalog implies routing/mapping is on (no
        // dedicated stored field), matching the load-time initialization.
        if matches!(self.app_type, AppType::Codex) {
            self.codex_local_routing_enabled = !self.codex_model_catalog.is_empty();
        }

        if !self.id_is_manual && !self.name.is_blank() {
            let id = crate::cli::commands::provider_input::generate_provider_id_for_app(
                &self.app_type,
                self.name.value.trim(),
                existing_ids,
            );
            self.id.set(id);
        }
    }

    fn reset_codex_local_routing_state(&mut self) {
        self.claude_api_format = ClaudeApiFormat::OpenAiResponses;
        self.codex_chat_reasoning = CodexChatReasoningConfig::default();
        self.codex_model_catalog.clear();
        self.codex_local_routing_enabled = false;
        self.codex_local_routing_field_idx = 0;
        self.codex_model_catalog_idx = 0;
        self.codex_model_catalog_field = CodexModelCatalogField::Model;
    }
}
