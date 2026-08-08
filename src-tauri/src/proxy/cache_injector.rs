use serde_json::{json, Value};

use super::types::OptimizerConfig;

pub fn inject(body: &mut Value, config: &OptimizerConfig) {
    if !config.cache_injection {
        return;
    }

    let existing = count_existing(body);
    upgrade_existing_ttl(body, &config.cache_ttl);

    let mut budget = 4usize.saturating_sub(existing);
    if budget == 0 {
        return;
    }

    if budget > 0 {
        if let Some(tools) = body.get_mut("tools").and_then(|value| value.as_array_mut()) {
            if let Some(last) = tools.last_mut() {
                if last.get("cache_control").is_none() {
                    if let Some(obj) = last.as_object_mut() {
                        obj.insert(
                            "cache_control".to_string(),
                            make_cache_control(&config.cache_ttl),
                        );
                        budget -= 1;
                    }
                }
            }
        }
    }

    if budget > 0 {
        if let Some(system_text) = body.get("system").and_then(|value| value.as_str()) {
            body["system"] = json!([{"type": "text", "text": system_text}]);
        }
        if let Some(system) = body
            .get_mut("system")
            .and_then(|value| value.as_array_mut())
        {
            if let Some(last) = system.last_mut() {
                if last.get("cache_control").is_none() {
                    if let Some(obj) = last.as_object_mut() {
                        obj.insert(
                            "cache_control".to_string(),
                            make_cache_control(&config.cache_ttl),
                        );
                        budget -= 1;
                    }
                }
            }
        }
    }

    if budget > 0 {
        if let Some(messages) = body
            .get_mut("messages")
            .and_then(|value| value.as_array_mut())
        {
            if let Some(assistant_msg) = messages.iter_mut().rev().find(|message| {
                message.get("role").and_then(|value| value.as_str()) == Some("assistant")
            }) {
                if let Some(content) = assistant_msg
                    .get_mut("content")
                    .and_then(|value| value.as_array_mut())
                {
                    if let Some(block) = content.iter_mut().rev().find(|block| {
                        let block_type = block
                            .get("type")
                            .and_then(|value| value.as_str())
                            .unwrap_or("");
                        block_type != "thinking" && block_type != "redacted_thinking"
                    }) {
                        if block.get("cache_control").is_none() {
                            if let Some(obj) = block.as_object_mut() {
                                obj.insert(
                                    "cache_control".to_string(),
                                    make_cache_control(&config.cache_ttl),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

fn make_cache_control(ttl: &str) -> Value {
    if ttl == "5m" {
        json!({"type": "ephemeral"})
    } else {
        json!({"type": "ephemeral", "ttl": ttl})
    }
}

fn count_existing(body: &Value) -> usize {
    let mut count = 0;

    if let Some(tools) = body.get("tools").and_then(|value| value.as_array()) {
        count += tools
            .iter()
            .filter(|tool| tool.get("cache_control").is_some())
            .count();
    }

    if let Some(system) = body.get("system").and_then(|value| value.as_array()) {
        count += system
            .iter()
            .filter(|block| block.get("cache_control").is_some())
            .count();
    }

    if let Some(messages) = body.get("messages").and_then(|value| value.as_array()) {
        for message in messages {
            if let Some(content) = message.get("content").and_then(|value| value.as_array()) {
                count += content
                    .iter()
                    .filter(|block| block.get("cache_control").is_some())
                    .count();
            }
        }
    }

    count
}

fn upgrade_existing_ttl(body: &mut Value, ttl: &str) {
    let upgrade = |value: &mut Value| {
        if let Some(cache_control) = value
            .get_mut("cache_control")
            .and_then(|cache_control| cache_control.as_object_mut())
        {
            if ttl == "5m" {
                cache_control.remove("ttl");
            } else {
                cache_control.insert("ttl".to_string(), json!(ttl));
            }
        }
    };

    if let Some(tools) = body.get_mut("tools").and_then(|value| value.as_array_mut()) {
        for tool in tools {
            upgrade(tool);
        }
    }

    if let Some(system) = body
        .get_mut("system")
        .and_then(|value| value.as_array_mut())
    {
        for block in system {
            upgrade(block);
        }
    }

    if let Some(messages) = body
        .get_mut("messages")
        .and_then(|value| value.as_array_mut())
    {
        for message in messages {
            if let Some(content) = message
                .get_mut("content")
                .and_then(|value| value.as_array_mut())
            {
                for block in content {
                    upgrade(block);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_config() -> OptimizerConfig {
        OptimizerConfig {
            enabled: true,
            thinking_optimizer: true,
            cache_injection: true,
            cache_ttl: "1h".to_string(),
        }
    }

    #[test]
    fn injects_breakpoints_into_tools_system_and_last_assistant_block() {
        let mut body = json!({
            "tools": [{"name": "tool_a"}],
            "system": [{"type": "text", "text": "sys"}],
            "messages": [{
                "role": "assistant",
                "content": [{"type": "text", "text": "hello"}]
            }]
        });

        inject(&mut body, &enabled_config());

        assert!(body["tools"][0].get("cache_control").is_some());
        assert!(body["system"][0].get("cache_control").is_some());
        assert!(body["messages"][0]["content"][0]
            .get("cache_control")
            .is_some());
    }

    #[test]
    fn ttl_5m_omits_ttl_field() {
        let config = OptimizerConfig {
            cache_ttl: "5m".to_string(),
            ..enabled_config()
        };
        let mut body = json!({"tools": [{"name": "tool_a"}]});

        inject(&mut body, &config);

        let cache_control = &body["tools"][0]["cache_control"];
        assert_eq!(cache_control["type"], "ephemeral");
        assert!(cache_control.get("ttl").is_none() || cache_control["ttl"].is_null());
    }
}
