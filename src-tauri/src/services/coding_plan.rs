//! 国产 Token Plan 额度查询服务
//!
//! 支持 Kimi For Coding、智谱 GLM、MiniMax 的 Token Plan 额度查询。
//! 复用 subscription 模块的 SubscriptionQuota / QuotaTier 类型。

use super::subscription::{CredentialStatus, QuotaTier, SubscriptionQuota};
use std::time::{SystemTime, UNIX_EPOCH};

// ── 供应商检测 ──────────────────────────────────────────────

enum CodingPlanProvider {
    Kimi,
    ZhipuCn,
    ZhipuEn,
    MiniMaxCn,
    MiniMaxEn,
}

fn detect_provider(base_url: &str) -> Option<CodingPlanProvider> {
    let url = base_url.to_lowercase();
    if url.contains("api.kimi.com/coding") {
        Some(CodingPlanProvider::Kimi)
    } else if url.contains("open.bigmodel.cn") || url.contains("bigmodel.cn") {
        Some(CodingPlanProvider::ZhipuCn)
    } else if url.contains("api.z.ai") {
        Some(CodingPlanProvider::ZhipuEn)
    } else if url.contains("api.minimaxi.com") {
        Some(CodingPlanProvider::MiniMaxCn)
    } else if url.contains("api.minimax.io") {
        Some(CodingPlanProvider::MiniMaxEn)
    } else {
        None
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn millis_to_iso8601(ms: i64) -> Option<String> {
    let secs = ms / 1000;
    let nsecs = ((ms % 1000) * 1_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, nsecs).map(|dt| dt.to_rfc3339())
}

/// 从 JSON 值提取重置时间，兼容字符串和数字格式
/// - 字符串：直接返回（ISO 8601）
/// - 数字：自动判断秒/毫秒并转为 ISO 8601
fn extract_reset_time(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = value.as_i64() {
        // 区分秒和毫秒：秒级时间戳 < 1e12，毫秒 >= 1e12
        let ms = if n < 1_000_000_000_000 { n * 1000 } else { n };
        return millis_to_iso8601(ms);
    }
    None
}

/// 解析 JSON 值为 f64，兼容数字和字符串格式（如 `100` 和 `"100"`）
fn parse_f64(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

fn make_error(msg: String) -> SubscriptionQuota {
    SubscriptionQuota {
        tool: "coding_plan".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: false,
        tiers: vec![],
        extra_usage: None,
        error: Some(msg),
        queried_at: Some(now_millis()),
    }
}

fn coding_plan_not_found(error: Option<String>) -> SubscriptionQuota {
    SubscriptionQuota {
        tool: "coding_plan".to_string(),
        credential_status: CredentialStatus::NotFound,
        credential_message: None,
        success: false,
        tiers: vec![],
        extra_usage: None,
        error,
        queried_at: None,
    }
}

// ── Kimi For Coding ─────────────────────────────────────────

async fn query_kimi(api_key: &str) -> Result<SubscriptionQuota, String> {
    let client = crate::proxy::http_client::get();

    let resp = client
        .get("https://api.kimi.com/coding/v1/usages")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(SubscriptionQuota {
            tool: "coding_plan".to_string(),
            credential_status: CredentialStatus::Expired,
            credential_message: Some("Invalid API key".to_string()),
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: Some(format!("Authentication failed (HTTP {status})")),
            queried_at: Some(now_millis()),
        });
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(make_error(format!("API error (HTTP {status}): {body}")));
    }

    let raw = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => return Err(format!("Failed to read response: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => return Ok(make_error(format!("Failed to parse response: {e}"))),
    };

    let mut tiers = Vec::new();

    // 5 小时窗口限额（优先显示）
    if let Some(limits) = body.get("limits").and_then(|v| v.as_array()) {
        for limit_item in limits {
            if let Some(detail) = limit_item.get("detail") {
                let limit = detail.get("limit").and_then(parse_f64).unwrap_or(1.0);
                let remaining = detail.get("remaining").and_then(parse_f64).unwrap_or(0.0);
                let resets_at = detail.get("resetTime").and_then(extract_reset_time);

                let used = (limit - remaining).max(0.0);
                let utilization = if limit > 0.0 {
                    (used / limit) * 100.0
                } else {
                    0.0
                };
                tiers.push(QuotaTier {
                    name: "five_hour".to_string(),
                    utilization,
                    resets_at,
                });
            }
        }
    }

    // 总体用量（周限额）
    if let Some(usage) = body.get("usage") {
        let limit = usage.get("limit").and_then(parse_f64).unwrap_or(1.0);
        let remaining = usage.get("remaining").and_then(parse_f64).unwrap_or(0.0);
        let resets_at = usage.get("resetTime").and_then(extract_reset_time);

        let used = (limit - remaining).max(0.0);
        let utilization = if limit > 0.0 {
            (used / limit) * 100.0
        } else {
            0.0
        };
        tiers.push(QuotaTier {
            name: "weekly_limit".to_string(),
            utilization,
            resets_at,
        });
    }

    Ok(SubscriptionQuota {
        tool: "coding_plan".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    })
}

// ── 智谱 GLM ────────────────────────────────────────────────

async fn query_zhipu(base_url: &str, api_key: &str) -> Result<SubscriptionQuota, String> {
    let client = crate::proxy::http_client::get();

    let quota_base = if base_url.to_ascii_lowercase().contains("bigmodel.cn") {
        "https://open.bigmodel.cn"
    } else {
        "https://api.z.ai"
    };
    let url = format!("{quota_base}/api/monitor/usage/quota/limit");
    let resp = client
        .get(url)
        .header("Authorization", api_key) // 注意：智谱不加 Bearer 前缀
        .header("Content-Type", "application/json")
        .header("Accept-Language", "en-US,en")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(SubscriptionQuota {
            tool: "coding_plan".to_string(),
            credential_status: CredentialStatus::Expired,
            credential_message: Some("Invalid API key".to_string()),
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: Some(format!("Authentication failed (HTTP {status})")),
            queried_at: Some(now_millis()),
        });
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(make_error(format!("API error (HTTP {status}): {body}")));
    }

    let raw = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => return Err(format!("Failed to read response: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => return Ok(make_error(format!("Failed to parse response: {e}"))),
    };

    // 检查业务级别错误
    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let msg = body
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Ok(make_error(format!("API error: {msg}")));
    }

    let data = match body.get("data") {
        Some(d) => d,
        None => return Ok(make_error("Missing 'data' field in response".to_string())),
    };

    let mut five_hour = None;
    let mut weekly = None;
    let mut fallback = Vec::new();

    if let Some(limits) = data.get("limits").and_then(|v| v.as_array()) {
        for limit_item in limits {
            let limit_type = limit_item
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let percentage = limit_item
                .get("percentage")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let next_reset = limit_item
                .get("nextResetTime")
                .and_then(|v| v.as_i64())
                .and_then(millis_to_iso8601);

            if !limit_type.eq_ignore_ascii_case("TOKENS_LIMIT") {
                continue;
            }

            let tier = (percentage, next_reset);
            match limit_item.get("unit").and_then(|value| value.as_i64()) {
                Some(3) if five_hour.is_none() => five_hour = Some(tier),
                Some(6) if weekly.is_none() => weekly = Some(tier),
                _ => fallback.push(tier),
            }
        }
    }

    for tier in fallback {
        if five_hour.is_none() {
            five_hour = Some(tier);
        } else if weekly.is_none() {
            weekly = Some(tier);
        }
    }
    let mut tiers = Vec::new();
    for (name, tier) in [("five_hour", five_hour), ("weekly_limit", weekly)] {
        if let Some((utilization, resets_at)) = tier {
            tiers.push(QuotaTier {
                name: name.to_string(),
                utilization,
                resets_at,
            });
        }
    }

    // 套餐等级存入 credential_message
    let level = data
        .get("level")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(SubscriptionQuota {
        tool: "coding_plan".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: level,
        success: true,
        tiers,
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    })
}

// ── MiniMax ─────────────────────────────────────────────────

async fn query_minimax(api_key: &str, is_cn: bool) -> Result<SubscriptionQuota, String> {
    let client = crate::proxy::http_client::get();

    let api_domain = if is_cn {
        "api.minimaxi.com"
    } else {
        "api.minimax.io"
    };
    let url = format!("https://{api_domain}/v1/api/openplatform/coding_plan/remains");

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(SubscriptionQuota {
            tool: "coding_plan".to_string(),
            credential_status: CredentialStatus::Expired,
            credential_message: Some("Invalid API key".to_string()),
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: Some(format!("Authentication failed (HTTP {status})")),
            queried_at: Some(now_millis()),
        });
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(make_error(format!("API error (HTTP {status}): {body}")));
    }

    let raw = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => return Err(format!("Failed to read response: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => return Ok(make_error(format!("Failed to parse response: {e}"))),
    };

    // 检查业务级别错误
    if let Some(base_resp) = body.get("base_resp") {
        let status_code = base_resp
            .get("status_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if status_code != 0 {
            let msg = base_resp
                .get("status_msg")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Ok(make_error(format!("API error (code {status_code}): {msg}")));
        }
    }

    let mut tiers = Vec::new();

    if let Some(model_remains) = body.get("model_remains").and_then(|v| v.as_array()) {
        // 只取第一个模型（MiniMax-M*，主力编程模型）
        if let Some(item) = model_remains.first() {
            // usage_count 是剩余量（满额=total，用完=0），需反转为已用百分比
            let interval_total = item
                .get("current_interval_total_count")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let interval_remaining = item
                .get("current_interval_usage_count")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let end_time = item.get("end_time").and_then(|v| v.as_i64());

            if interval_total > 0.0 {
                tiers.push(QuotaTier {
                    name: "five_hour".to_string(),
                    utilization: ((interval_total - interval_remaining) / interval_total) * 100.0,
                    resets_at: end_time.and_then(millis_to_iso8601),
                });
            }

            // 周额度
            let weekly_total = item
                .get("current_weekly_total_count")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let weekly_remaining = item
                .get("current_weekly_usage_count")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let weekly_end = item.get("weekly_end_time").and_then(|v| v.as_i64());

            if weekly_total > 0.0 {
                tiers.push(QuotaTier {
                    name: "weekly_limit".to_string(),
                    utilization: ((weekly_total - weekly_remaining) / weekly_total) * 100.0,
                    resets_at: weekly_end.and_then(millis_to_iso8601),
                });
            }
        }
    }

    Ok(SubscriptionQuota {
        tool: "coding_plan".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    })
}

// ── 公开入口 ────────────────────────────────────────────────

const ZHIPU_TEAM_QUOTA_URL: &str = "https://open.bigmodel.cn/api/monitor/usage/quota/limit";

async fn query_zhipu_team(
    api_key: &str,
    organization_id: &str,
    project_id: &str,
) -> Result<SubscriptionQuota, String> {
    query_zhipu_team_at(ZHIPU_TEAM_QUOTA_URL, api_key, organization_id, project_id).await
}

async fn query_zhipu_team_at(
    quota_url_base: &str,
    api_key: &str,
    organization_id: &str,
    project_id: &str,
) -> Result<SubscriptionQuota, String> {
    let response = crate::proxy::http_client::get()
        .get(format!("{quota_url_base}?type=2"))
        .header("Authorization", api_key)
        .header("bigmodel-organization", organization_id)
        .header("bigmodel-project", project_id)
        .header("Content-Type", "application/json")
        .header("Accept-Language", "en-US,en")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| format!("Network error: {error}"))?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(SubscriptionQuota {
            tool: "coding_plan".to_string(),
            credential_status: CredentialStatus::Expired,
            credential_message: Some("Invalid API key".to_string()),
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: Some(format!("Authentication failed (HTTP {status})")),
            queried_at: Some(now_millis()),
        });
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Ok(make_error(format!("API error (HTTP {status}): {body}")));
    }

    let raw = response
        .bytes()
        .await
        .map_err(|error| format!("Failed to read response: {error}"))?;
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(body) => body,
        Err(error) => return Ok(make_error(format!("Failed to parse response: {error}"))),
    };

    if body.get("success").and_then(|value| value.as_bool()) == Some(false) {
        let message = body
            .get("msg")
            .and_then(|value| value.as_str())
            .unwrap_or("Unknown error");
        return Ok(make_error(format!("API error: {message}")));
    }

    let Some(data) = body.get("data") else {
        return Ok(make_error("Missing 'data' field in response".to_string()));
    };
    let mut quota = SubscriptionQuota {
        tool: "coding_plan".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: data
            .get("level")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        success: true,
        tiers: vec![],
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    };
    let personal_shape = serde_json::json!({ "success": true, "data": data });
    if let Some(data) = personal_shape.get("data") {
        let mut five_hour = None;
        let mut weekly = None;
        if let Some(limits) = data.get("limits").and_then(|value| value.as_array()) {
            for item in limits {
                if !item
                    .get("type")
                    .and_then(|value| value.as_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("TOKENS_LIMIT"))
                {
                    continue;
                }
                let entry = (
                    item.get("percentage")
                        .and_then(|value| value.as_f64())
                        .unwrap_or(0.0),
                    item.get("nextResetTime")
                        .and_then(|value| value.as_i64())
                        .and_then(millis_to_iso8601),
                );
                match item.get("unit").and_then(|value| value.as_i64()) {
                    Some(6) => weekly = Some(entry),
                    _ => five_hour = Some(entry),
                }
            }
        }
        for (name, entry) in [("five_hour", five_hour), ("weekly_limit", weekly)] {
            if let Some((utilization, resets_at)) = entry {
                quota.tiers.push(QuotaTier {
                    name: name.to_string(),
                    utilization,
                    resets_at,
                });
            }
        }
    }
    Ok(quota)
}

pub async fn get_coding_plan_quota(
    base_url: &str,
    api_key: &str,
    coding_plan_provider: Option<&str>,
    team_organization_id: Option<&str>,
    team_project_id: Option<&str>,
) -> Result<SubscriptionQuota, String> {
    if coding_plan_provider.is_some_and(|value| value.eq_ignore_ascii_case("zhipu_team")) {
        let organization_id = team_organization_id.unwrap_or("").trim();
        let project_id = team_project_id.unwrap_or("").trim();
        if api_key.trim().is_empty() || organization_id.is_empty() || project_id.is_empty() {
            return Ok(coding_plan_not_found(Some(
                "Zhipu team plan needs the API key + organization ID + project ID".to_string(),
            )));
        }
        return query_zhipu_team(api_key, organization_id, project_id).await;
    }

    if api_key.trim().is_empty() {
        return Ok(coding_plan_not_found(None));
    }

    let provider = match detect_provider(base_url) {
        Some(p) => p,
        None => {
            return Ok(coding_plan_not_found(Some(format!(
                "No supported coding-plan quota endpoint for {base_url}"
            ))))
        }
    };

    let quota = match provider {
        CodingPlanProvider::Kimi => query_kimi(api_key).await,
        CodingPlanProvider::ZhipuCn | CodingPlanProvider::ZhipuEn => {
            query_zhipu(base_url, api_key).await
        }
        CodingPlanProvider::MiniMaxCn => query_minimax(api_key, true).await,
        CodingPlanProvider::MiniMaxEn => query_minimax(api_key, false).await,
    };

    quota
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn ensure_no_proxy_for_loopback() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
            std::env::set_var("no_proxy", "127.0.0.1,localhost");
        });
    }

    #[tokio::test]
    async fn zhipu_team_requires_all_credentials_before_network_io() {
        let quota = get_coding_plan_quota(
            "https://open.bigmodel.cn/api/coding",
            "key",
            Some("zhipu_team"),
            Some("org"),
            None,
        )
        .await
        .expect("deterministic missing credential result");
        assert!(!quota.success);
        assert!(matches!(
            quota.credential_status,
            CredentialStatus::NotFound
        ));
        assert!(quota
            .error
            .as_deref()
            .is_some_and(|message| message.contains("organization ID + project ID")));
    }

    #[tokio::test]
    async fn zhipu_team_request_uses_type2_and_team_headers() {
        ensure_no_proxy_for_loopback();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let address = listener.local_addr().expect("listener address");
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let captured_server = captured.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 2048];
            while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).expect("read request");
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..read]);
            }
            *captured_server.lock().expect("capture lock") =
                String::from_utf8_lossy(&bytes).to_string();
            let body = serde_json::json!({
                "success": true,
                "data": {
                    "level": "team",
                    "limits": [
                        {"type": "TOKENS_LIMIT", "unit": 3, "percentage": 25.0},
                        {"type": "TOKENS_LIMIT", "unit": 6, "percentage": 50.0}
                    ]
                }
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });

        let quota = query_zhipu_team_at(
            &format!("http://{address}/quota"),
            "team-key",
            "org-id",
            "project-id",
        )
        .await
        .expect("team quota request");
        server.join().expect("server thread");

        let request = captured.lock().expect("capture lock").to_ascii_lowercase();
        assert!(request.contains("/quota?type=2"));
        assert!(request.contains("authorization: team-key"));
        assert!(request.contains("bigmodel-organization: org-id"));
        assert!(request.contains("bigmodel-project: project-id"));
        assert_eq!(quota.tiers.len(), 2);
        assert_eq!(quota.tiers[0].name, "five_hour");
        assert_eq!(quota.tiers[1].name, "weekly_limit");
    }

    #[tokio::test]
    async fn zhipu_team_transport_failure_stays_in_error_channel() {
        ensure_no_proxy_for_loopback();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let address = listener.local_addr().expect("listener address");
        drop(listener);

        let error =
            query_zhipu_team_at(&format!("http://{address}/quota"), "key", "org", "project")
                .await
                .expect_err("connection failure must remain transient");
        assert!(error.contains("Network error"));
    }
}
