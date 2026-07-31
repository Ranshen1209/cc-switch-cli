use std::collections::{HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection};

use crate::database::usage_prune_watermark::read_usage_prune_high_watermark;
use crate::error::AppError;
use crate::services::sql_helpers::fresh_input_sql;
use crate::services::usage_stats::effective_usage_log_filter;
use crate::session_manager::{
    SessionCostCoverage, SessionCostKind, SessionCreatedAtKind, SessionUsageSummary,
};

use super::{QueryControl, SessionCostIdentity, SessionCostTarget};

#[derive(Debug)]
struct Aggregate {
    provider_id: String,
    logical_session_id: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    total_cost: f64,
    has_priced_row: bool,
    has_unpriced_row: bool,
    last_modified: Option<i64>,
}

struct AggregateQuery {
    sql: String,
    values: Vec<Value>,
}

pub(crate) fn project_main_connection(
    conn: &Connection,
    targets: &[SessionCostTarget],
    control: &QueryControl,
) -> Result<HashMap<SessionCostIdentity, SessionUsageSummary>, AppError> {
    if targets.is_empty() || control.is_cancelled() {
        return Ok(HashMap::new());
    }
    let targets = super::unambiguous_targets(targets);
    if targets.is_empty() {
        return Ok(HashMap::new());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| AppError::Database(format!("begin session cost snapshot: {error}")))?;
    let watermark = read_usage_prune_high_watermark(&tx)?;
    let aggregates = query_aggregates(&tx, &targets, control)?;
    if control.is_cancelled() {
        return Err(AppError::Message(
            "session cost projection cancelled".to_string(),
        ));
    }

    let mut by_logical = HashMap::new();
    for aggregate in aggregates {
        by_logical.insert(
            (
                aggregate.provider_id.clone(),
                aggregate.logical_session_id.clone(),
            ),
            aggregate,
        );
    }

    let mut overlays = HashMap::new();
    for target in &targets {
        let key = (
            target.identity.provider_id.clone(),
            target.identity.session_id.clone(),
        );
        let Some(aggregate) = by_logical.get(&key) else {
            continue;
        };
        let cost = aggregate
            .has_priced_row
            .then_some(aggregate.total_cost)
            .filter(|value| value.is_finite() && *value >= 0.0);
        let mut coverage = classify_coverage(target, aggregate.last_modified, watermark);
        if aggregate.has_unpriced_row || (aggregate.has_priced_row && cost.is_none()) {
            coverage = cap_partial(coverage);
        }
        overlays.insert(
            target.identity.clone(),
            SessionUsageSummary {
                input_tokens: aggregate.input_tokens,
                output_tokens: aggregate.output_tokens,
                cache_read_tokens: aggregate.cache_read_tokens,
                cache_creation_tokens: aggregate.cache_creation_tokens,
                cost,
                cost_kind: SessionCostKind::Recorded,
                coverage,
            },
        );
    }

    tx.commit()
        .map_err(|error| AppError::Database(format!("end session cost snapshot: {error}")))?;
    Ok(overlays)
}

fn query_aggregates(
    conn: &Connection,
    targets: &[SessionCostTarget],
    control: &QueryControl,
) -> Result<Vec<Aggregate>, AppError> {
    let query = build_aggregate_query(targets);
    if query.values.is_empty() {
        return Ok(Vec::new());
    }

    if control.is_cancelled() {
        return Err(AppError::Message(
            "session cost projection cancelled".to_string(),
        ));
    }
    let mut statement = conn.prepare(&query.sql).map_err(AppError::from)?;
    let rows = statement
        .query_map(params_from_iter(query.values), |row| {
            Ok(Aggregate {
                provider_id: row.get(0)?,
                logical_session_id: row.get(1)?,
                input_tokens: nonnegative(row.get(2)?),
                output_tokens: nonnegative(row.get(3)?),
                cache_read_tokens: nonnegative(row.get(4)?),
                cache_creation_tokens: nonnegative(row.get(5)?),
                total_cost: row.get(6)?,
                has_priced_row: row.get::<_, i64>(7)? != 0,
                has_unpriced_row: row.get::<_, i64>(8)? != 0,
                last_modified: row.get(9)?,
            })
        })
        .map_err(AppError::from)?;

    let mut aggregates = Vec::new();
    for row in rows {
        if control.is_cancelled() {
            return Err(AppError::Message(
                "session cost projection cancelled".to_string(),
            ));
        }
        aggregates.push(row.map_err(AppError::from)?);
    }
    Ok(aggregates)
}

fn build_aggregate_query(targets: &[SessionCostTarget]) -> AggregateQuery {
    let mut wanted = Vec::<(String, String, String, String)>::new();
    let mut seen = HashSet::new();
    for target in targets {
        let provider = target.identity.provider_id.as_str();
        if !matches!(provider, "claude" | "codex" | "gemini" | "opencode") {
            continue;
        }
        let sync_key = sync_key(target);
        let direct = (
            provider.to_string(),
            target.identity.session_id.clone(),
            target.identity.session_id.clone(),
            sync_key.clone(),
        );
        if seen.insert(direct.clone()) {
            wanted.push(direct);
        }
        if provider == "codex" {
            let alias = (
                provider.to_string(),
                format!("codex_{}", target.identity.session_id),
                target.identity.session_id.clone(),
                sync_key,
            );
            if seen.insert(alias.clone()) {
                wanted.push(alias);
            }
        }
    }
    if wanted.is_empty() {
        return AggregateQuery {
            sql: String::new(),
            values: Vec::new(),
        };
    }

    let values_sql = std::iter::repeat_n("(?, ?, ?, ?)", wanted.len())
        .collect::<Vec<_>>()
        .join(", ");
    let fresh_input = fresh_input_sql("l");
    let effective_filter = effective_usage_log_filter("l");
    let row_tokens = format!(
        "({fresh_input} + l.output_tokens + l.cache_read_tokens + l.cache_creation_tokens)"
    );
    let true_zero_pricing = "EXISTS (
        SELECT 1
        FROM model_pricing zero_pricing
        WHERE LOWER(zero_pricing.model_id) =
              LOWER(COALESCE(l.pricing_model, l.model, ''))
          AND CAST(zero_pricing.input_cost_per_million AS REAL) = 0
          AND CAST(zero_pricing.output_cost_per_million AS REAL) = 0
          AND CAST(zero_pricing.cache_read_cost_per_million AS REAL) = 0
          AND CAST(zero_pricing.cache_creation_cost_per_million AS REAL) = 0
    )";
    let unpriced = format!(
        "{row_tokens} > 0 AND (
            TRIM(l.pricing_model) = ''
            OR (
                CAST(l.total_cost_usd AS REAL) = 0
                AND NOT {true_zero_pricing}
            )
        )"
    );
    let sql = format!(
        "WITH wanted(provider_id, lookup_session_id, logical_session_id, sync_key) AS (
             VALUES {values_sql}
         )
         SELECT
             w.provider_id,
             w.logical_session_id,
             COALESCE(SUM({fresh_input}), 0),
             COALESCE(SUM(l.output_tokens), 0),
             COALESCE(SUM(l.cache_read_tokens), 0),
             COALESCE(SUM(l.cache_creation_tokens), 0),
             COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0.0),
             MAX(CASE WHEN {unpriced} THEN 0
                      WHEN {row_tokens} > 0 THEN 1 ELSE 0 END),
             MAX(CASE WHEN {unpriced} THEN 1 ELSE 0 END),
             MAX(sync.last_modified)
         FROM wanted w
         JOIN proxy_request_logs l INDEXED BY idx_request_logs_session
           ON l.session_id = w.lookup_session_id
          AND l.app_type = w.provider_id
         LEFT JOIN session_log_sync sync ON sync.file_path = w.sync_key
         WHERE {effective_filter}
         GROUP BY w.provider_id, w.logical_session_id"
    );

    let mut values = Vec::with_capacity(wanted.len() * 4);
    for (provider, lookup, logical, sync_key) in wanted {
        values.push(Value::Text(provider));
        values.push(Value::Text(lookup));
        values.push(Value::Text(logical));
        values.push(Value::Text(sync_key));
    }

    AggregateQuery { sql, values }
}

#[cfg(test)]
pub(super) fn explain_main_query_plan(
    conn: &Connection,
    targets: &[SessionCostTarget],
) -> Result<Vec<String>, AppError> {
    let query = build_aggregate_query(targets);
    if query.values.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {}", query.sql))
        .map_err(AppError::from)?;
    let rows = statement
        .query_map(params_from_iter(query.values), |row| {
            row.get::<_, String>(3)
        })
        .map_err(AppError::from)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

fn sync_key(target: &SessionCostTarget) -> String {
    let path = target.identity.source_path.as_deref().unwrap_or_default();
    if target.identity.provider_id == "opencode" {
        path.strip_prefix("sqlite:").unwrap_or(path).to_string()
    } else {
        path.to_string()
    }
}

fn classify_coverage(
    target: &SessionCostTarget,
    last_modified: Option<i64>,
    watermark: Option<crate::database::usage_prune_watermark::UsagePruneHighWatermark>,
) -> SessionCostCoverage {
    if target.created_at.is_none()
        || target.source_mtime_ns.is_none()
        || target.created_at_kind.is_none()
    {
        return SessionCostCoverage::Unknown;
    }

    let provider_can_complete = matches!(target.identity.provider_id.as_str(), "codex" | "gemini");
    let prune_complete = watermark.is_some_and(|watermark| {
        watermark.history_complete
            && target
                .created_at
                .is_some_and(|created_at| created_at >= watermark.epoch.saturating_mul(1_000))
    });
    let trusted_created_at =
        target.created_at_kind == Some(SessionCreatedAtKind::ProviderTimestamp);
    let source_complete = last_modified
        .zip(target.source_mtime_ns)
        .is_some_and(|(synced, source)| synced >= source);

    if provider_can_complete && prune_complete && trusted_created_at && source_complete {
        SessionCostCoverage::Complete
    } else {
        SessionCostCoverage::Partial
    }
}

fn cap_partial(coverage: SessionCostCoverage) -> SessionCostCoverage {
    match coverage {
        SessionCostCoverage::Complete => SessionCostCoverage::Partial,
        other => other,
    }
}

fn nonnegative(value: i64) -> u64 {
    value.max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::{sync_key, SessionCostIdentity, SessionCostTarget};

    #[test]
    fn opencode_sqlite_source_maps_to_the_per_session_sync_key() {
        let target = SessionCostTarget {
            identity: SessionCostIdentity {
                provider_id: "opencode".to_string(),
                session_id: "ses_123".to_string(),
                source_path: Some("sqlite:/tmp/opencode.db:ses_123".to_string()),
            },
            created_at: None,
            source_mtime_ns: None,
            created_at_kind: None,
        };
        assert_eq!(sync_key(&target), "/tmp/opencode.db:ses_123");
    }
}
