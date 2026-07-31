use std::collections::{HashMap, HashSet};
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection, OpenFlags};

use crate::error::AppError;
use crate::session_manager::{SessionCostCoverage, SessionCostKind, SessionUsageSummary};

use super::{QueryControl, SessionCostIdentity, SessionCostTarget};

pub(super) fn project(
    targets: &[SessionCostTarget],
    control: &QueryControl,
) -> Result<HashMap<SessionCostIdentity, SessionUsageSummary>, AppError> {
    let path = crate::hermes_config::get_hermes_dir().join("state.db");
    if !path.exists() || control.is_cancelled() {
        return Ok(HashMap::new());
    }
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(AppError::from)?;
    conn.busy_timeout(Duration::from_millis(250))
        .map_err(AppError::from)?;
    conn.pragma_update(None, "query_only", true)
        .map_err(AppError::from)?;
    control.install_progress_handler(&conn);
    let result = project_connection(&conn, targets, control);
    QueryControl::clear_progress_handler(&conn);
    result
}

fn project_connection(
    conn: &Connection,
    targets: &[SessionCostTarget],
    control: &QueryControl,
) -> Result<HashMap<SessionCostIdentity, SessionUsageSummary>, AppError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| AppError::Database(format!("begin Hermes cost snapshot: {error}")))?;
    let overlays = project_snapshot(&tx, targets, control)?;
    tx.commit()
        .map_err(|error| AppError::Database(format!("end Hermes cost snapshot: {error}")))?;
    Ok(overlays)
}

fn project_snapshot(
    conn: &Connection,
    targets: &[SessionCostTarget],
    control: &QueryControl,
) -> Result<HashMap<SessionCostIdentity, SessionUsageSummary>, AppError> {
    let wanted = targets
        .iter()
        .map(|target| target.identity.session_id.clone())
        .collect::<HashSet<_>>();
    let mut combined = HashMap::<String, UsageBuckets>::new();

    if table_exists(conn, "session_model_usage")? {
        let columns = table_columns(conn, "session_model_usage")?;
        if columns.contains("session_id") {
            for (id, usage) in query_table(
                conn,
                "session_model_usage",
                "session_id",
                &columns,
                &wanted,
                control,
            )? {
                combined.insert(id, usage);
            }
        }
    }
    if table_exists(conn, "sessions")? {
        let columns = table_columns(conn, "sessions")?;
        if columns.contains("id") {
            for (id, usage) in query_table(conn, "sessions", "id", &columns, &wanted, control)? {
                combined
                    .entry(id)
                    .and_modify(|current| *current = current.maximum(usage))
                    .or_insert(usage);
            }
        }
    }

    let mut overlays = HashMap::new();
    for target in targets {
        let Some(usage) = combined.get(&target.identity.session_id).copied() else {
            continue;
        };
        if usage.is_empty() {
            continue;
        }
        overlays.insert(target.identity.clone(), usage.into_summary());
    }
    Ok(overlays)
}

fn query_table(
    conn: &Connection,
    table: &str,
    id_column: &str,
    columns: &HashSet<String>,
    wanted: &HashSet<String>,
    control: &QueryControl,
) -> Result<Vec<(String, UsageBuckets)>, AppError> {
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let token = |name: &str| {
        if columns.contains(name) {
            format!("COALESCE(SUM(source.{name}), 0)")
        } else {
            "0".to_string()
        }
    };
    let cost = if columns.contains("estimated_cost_usd") {
        "SUM(source.estimated_cost_usd)"
    } else {
        "NULL"
    };
    let values = std::iter::repeat_n("(?)", wanted.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "WITH wanted(session_id) AS (VALUES {values})
         SELECT wanted.session_id,
                {} AS input_tokens,
                {} AS output_tokens,
                {} AS cache_read_tokens,
                {} AS cache_write_tokens,
                {cost} AS estimated_cost_usd
         FROM wanted
         JOIN {table} source ON source.{id_column} = wanted.session_id
         GROUP BY wanted.session_id",
        token("input_tokens"),
        token("output_tokens"),
        token("cache_read_tokens"),
        token("cache_write_tokens"),
    );
    let bindings = wanted.iter().cloned().map(Value::Text).collect::<Vec<_>>();
    let mut statement = conn.prepare(&sql).map_err(AppError::from)?;
    let rows = statement
        .query_map(params_from_iter(bindings), |row| {
            Ok((
                row.get::<_, String>(0)?,
                UsageBuckets {
                    input_tokens: nonnegative(row.get(1)?),
                    output_tokens: nonnegative(row.get(2)?),
                    cache_read_tokens: nonnegative(row.get(3)?),
                    cache_creation_tokens: nonnegative(row.get(4)?),
                    estimated_cost_usd: valid_cost(row.get(5)?),
                },
            ))
        })
        .map_err(AppError::from)?;
    let mut result = Vec::new();
    for row in rows {
        if control.is_cancelled() {
            return Err(AppError::Message(
                "Hermes session cost projection cancelled".to_string(),
            ));
        }
        result.push(row.map_err(AppError::from)?);
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, Default)]
struct UsageBuckets {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    estimated_cost_usd: Option<f64>,
}

impl UsageBuckets {
    fn maximum(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.max(other.input_tokens),
            output_tokens: self.output_tokens.max(other.output_tokens),
            cache_read_tokens: self.cache_read_tokens.max(other.cache_read_tokens),
            cache_creation_tokens: self.cache_creation_tokens.max(other.cache_creation_tokens),
            estimated_cost_usd: match (self.estimated_cost_usd, other.estimated_cost_usd) {
                (Some(left), Some(right)) => Some(left.max(right)),
                (Some(value), None) | (None, Some(value)) => Some(value),
                (None, None) => None,
            },
        }
    }

    fn is_empty(self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cache_read_tokens == 0
            && self.cache_creation_tokens == 0
    }

    fn into_summary(self) -> SessionUsageSummary {
        SessionUsageSummary {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_creation_tokens: self.cache_creation_tokens,
            cost: self.estimated_cost_usd,
            cost_kind: SessionCostKind::Estimated,
            coverage: SessionCostCoverage::Unknown,
        }
    }
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
        [table],
        |row| row.get(0),
    )
    .map_err(AppError::from)
}

fn table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>, AppError> {
    let mut statement = conn
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .map_err(AppError::from)?;
    let rows = statement
        .query_map([table], |row| row.get::<_, String>(0))
        .map_err(AppError::from)?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(AppError::from)
}

fn nonnegative(value: i64) -> u64 {
    value.max(0) as u64
}

fn valid_cost(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use rusqlite::Connection;

    use crate::services::session_cost::{QueryControl, SessionCostIdentity, SessionCostTarget};
    use crate::session_manager::{SessionCostCoverage, SessionCostKind};

    use super::project_connection;

    fn control() -> QueryControl {
        QueryControl {
            active_cost_seq: Arc::new(AtomicU64::new(1)),
            cost_seq: 1,
            deadline: Instant::now() + Duration::from_secs(2),
        }
    }

    fn target(session_id: &str) -> SessionCostTarget {
        SessionCostTarget {
            identity: SessionCostIdentity {
                provider_id: "hermes".to_string(),
                session_id: session_id.to_string(),
                source_path: Some(format!("state.db#{session_id}")),
            },
            created_at: None,
            source_mtime_ns: None,
            created_at_kind: None,
        }
    }

    #[test]
    fn wanted_sessions_are_projected_as_estimates_from_hermes_state() {
        let conn = Connection::open_in_memory().expect("Hermes fixture");
        conn.execute_batch(
            "CREATE TABLE session_model_usage (
                 session_id TEXT NOT NULL,
                 input_tokens INTEGER NOT NULL,
                 output_tokens INTEGER NOT NULL,
                 cache_read_tokens INTEGER NOT NULL,
                 cache_write_tokens INTEGER NOT NULL,
                 estimated_cost_usd REAL NOT NULL
             );
             INSERT INTO session_model_usage VALUES
                 ('wanted', 10, 20, 30, 40, 0.5),
                 ('wanted', 1, 2, 3, 4, 0.25),
                 ('unrelated', 999, 999, 999, 999, 99.0);",
        )
        .expect("seed Hermes fixture");

        let wanted = target("wanted");
        let overlays = project_connection(&conn, std::slice::from_ref(&wanted), &control())
            .expect("project Hermes state");
        assert_eq!(overlays.len(), 1);
        let usage = overlays
            .get(&wanted.identity)
            .expect("wanted Hermes overlay");
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_creation_tokens,
            ),
            (11, 22, 33, 44)
        );
        assert_eq!(usage.cost, Some(0.75));
        assert_eq!(usage.cost_kind, SessionCostKind::Estimated);
        assert_eq!(usage.coverage, SessionCostCoverage::Unknown);
    }

    #[test]
    fn token_data_without_a_cost_column_keeps_cost_unavailable() {
        let conn = Connection::open_in_memory().expect("Hermes fixture");
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT NOT NULL,
                 input_tokens INTEGER NOT NULL,
                 output_tokens INTEGER NOT NULL
             );
             INSERT INTO sessions VALUES ('wanted', 10, 5);",
        )
        .expect("seed Hermes fixture");

        let wanted = target("wanted");
        let overlays = project_connection(&conn, std::slice::from_ref(&wanted), &control())
            .expect("project Hermes state");
        let usage = overlays
            .get(&wanted.identity)
            .expect("wanted Hermes overlay");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.cost, None);
        assert_eq!(usage.cost_kind, SessionCostKind::Estimated);
    }

    #[test]
    fn actual_cost_column_is_not_relabelled_as_an_estimate() {
        let conn = Connection::open_in_memory().expect("Hermes fixture");
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT NOT NULL,
                 input_tokens INTEGER NOT NULL,
                 output_tokens INTEGER NOT NULL,
                 actual_cost_usd REAL NOT NULL
             );
             INSERT INTO sessions VALUES ('wanted', 10, 5, 0.75);",
        )
        .expect("seed Hermes fixture");

        let wanted = target("wanted");
        let overlays = project_connection(&conn, std::slice::from_ref(&wanted), &control())
            .expect("project Hermes state");
        let usage = overlays
            .get(&wanted.identity)
            .expect("wanted Hermes overlay");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(
            usage.cost, None,
            "only the source's estimated_cost_usd field may render with ~"
        );
        assert_eq!(usage.cost_kind, SessionCostKind::Estimated);
    }

    #[test]
    fn null_estimated_cost_is_not_coalesced_into_a_zero_estimate() {
        let conn = Connection::open_in_memory().expect("Hermes fixture");
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT NOT NULL,
                 input_tokens INTEGER NOT NULL,
                 output_tokens INTEGER NOT NULL,
                 estimated_cost_usd REAL
             );
             INSERT INTO sessions VALUES ('wanted', 10, 5, NULL);",
        )
        .expect("seed Hermes fixture");

        let wanted = target("wanted");
        let overlays = project_connection(&conn, std::slice::from_ref(&wanted), &control())
            .expect("project Hermes state");
        let usage = overlays
            .get(&wanted.identity)
            .expect("wanted Hermes overlay");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(
            usage.cost, None,
            "missing source estimates must render as unavailable, not ~$0"
        );
    }

    #[test]
    fn cost_only_rows_have_no_displayable_usage_without_token_bearing_rows() {
        let conn = Connection::open_in_memory().expect("Hermes fixture");
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT NOT NULL,
                 estimated_cost_usd REAL
             );
             INSERT INTO sessions VALUES
                 ('positive', 0.75),
                 ('zero', 0.0),
                 ('missing', NULL);",
        )
        .expect("seed Hermes fixture");

        let targets = [target("positive"), target("zero"), target("missing")];
        let overlays =
            project_connection(&conn, &targets, &control()).expect("project Hermes state");

        assert!(
            overlays.is_empty(),
            "Hermes estimates require at least one token-bearing attributable row"
        );
    }
}
