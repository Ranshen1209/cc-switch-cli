pub(super) fn update_toml_base_url(toml_str: &str, new_url: &str) -> String {
    use toml_edit::DocumentMut;

    let mut doc = match toml_str.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => return toml_str.to_string(),
    };

    let model_provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::to_string);

    if let Some(provider_key) = model_provider {
        if doc.get("model_providers").is_none() {
            doc["model_providers"] = toml_edit::table();
        }

        if let Some(model_providers) = doc["model_providers"].as_table_mut() {
            if !model_providers.contains_key(&provider_key) {
                model_providers[&provider_key] = toml_edit::table();
            }

            if let Some(provider_table) = model_providers[&provider_key].as_table_mut() {
                provider_table["base_url"] = toml_edit::value(new_url);
                return doc.to_string();
            }
        }
    }

    doc["base_url"] = toml_edit::value(new_url);
    doc.to_string()
}

pub(super) fn remove_loopback_base_url_from_toml(toml_str: &str) -> String {
    use toml_edit::DocumentMut;

    let mut doc = match toml_str.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => return toml_str.to_string(),
    };

    let model_provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::to_string);

    if let Some(provider_key) = model_provider {
        if let Some(base_url) = doc
            .get("model_providers")
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get(&provider_key))
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get("base_url"))
            .and_then(|item| item.as_str())
        {
            if contains_loopback_proxy_url(base_url) {
                if let Some(section) = doc
                    .get_mut("model_providers")
                    .and_then(|item| item.as_table_like_mut())
                    .and_then(|table| table.get_mut(&provider_key))
                    .and_then(|item| item.as_table_like_mut())
                {
                    section.remove("base_url");
                }
            }
        }
    }

    if doc
        .get("base_url")
        .and_then(|item| item.as_str())
        .is_some_and(contains_loopback_proxy_url)
    {
        doc.as_table_mut().remove("base_url");
    }

    doc.to_string()
}

pub(super) fn is_loopback_proxy_url(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost") || url.contains("[::1]")
}

pub(super) fn contains_loopback_proxy_url(text: &str) -> bool {
    text.contains("127.0.0.1") || text.contains("localhost") || text.contains("[::1]")
}
