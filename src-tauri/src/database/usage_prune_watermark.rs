use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub(crate) const USAGE_PRUNE_HIGH_WATERMARK_KEY: &str = "usage_prune_high_watermark";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UsagePruneHighWatermark {
    pub(crate) epoch: i64,
    pub(crate) history_complete: bool,
}

pub(crate) fn read_usage_prune_high_watermark(
    conn: &Connection,
) -> Result<Option<UsagePruneHighWatermark>, AppError> {
    let value = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [USAGE_PRUNE_HIGH_WATERMARK_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| AppError::Database(format!("read usage prune watermark: {error}")))?;
    let Some(value) = value else {
        return Ok(None);
    };
    let watermark = serde_json::from_str::<UsagePruneHighWatermark>(&value).ok();
    Ok(watermark.filter(|watermark| watermark.epoch >= 0))
}

fn write_usage_prune_high_watermark(
    conn: &Connection,
    watermark: UsagePruneHighWatermark,
) -> Result<(), AppError> {
    let value = serde_json::to_string(&watermark)
        .map_err(|error| AppError::Database(format!("serialize usage prune watermark: {error}")))?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![USAGE_PRUNE_HIGH_WATERMARK_KEY, value],
    )
    .map_err(|error| AppError::Database(format!("write usage prune watermark: {error}")))?;
    Ok(())
}

/// Seed history provenance without changing the database schema.
///
/// A pre-existing database can have pruned data from older binaries, so a
/// missing or malformed value is permanently conservative. Only a truly fresh
/// database begins with complete history.
pub(crate) fn initialize_usage_prune_high_watermark(
    conn: &Connection,
    is_new_database: bool,
) -> Result<(), AppError> {
    if read_usage_prune_high_watermark(conn)?.is_some() {
        return Ok(());
    }
    write_usage_prune_high_watermark(
        conn,
        UsagePruneHighWatermark {
            epoch: 0,
            history_complete: is_new_database,
        },
    )
}

/// Advance the cutoff inside the caller's rollup/prune savepoint.
///
/// Missing or damaged provenance is repaired as incomplete and can never be
/// promoted by a later successful prune.
pub(crate) fn advance_usage_prune_high_watermark(
    conn: &Connection,
    epoch: i64,
) -> Result<(), AppError> {
    let current = read_usage_prune_high_watermark(conn)?.unwrap_or(UsagePruneHighWatermark {
        epoch: 0,
        history_complete: false,
    });
    write_usage_prune_high_watermark(
        conn,
        UsagePruneHighWatermark {
            epoch: current.epoch.max(epoch),
            history_complete: current.history_complete,
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::database::Database;

    use super::{
        advance_usage_prune_high_watermark, initialize_usage_prune_high_watermark,
        read_usage_prune_high_watermark, UsagePruneHighWatermark, USAGE_PRUNE_HIGH_WATERMARK_KEY,
    };

    #[test]
    fn fresh_and_existing_databases_start_with_different_history_evidence() {
        for (is_new_database, expected_complete) in [(true, true), (false, false)] {
            let db = Database::memory().expect("memory database");
            let conn = db.conn.lock().expect("database lock");
            conn.execute(
                "DELETE FROM settings WHERE key = ?1",
                [USAGE_PRUNE_HIGH_WATERMARK_KEY],
            )
            .expect("clear initializer value");
            initialize_usage_prune_high_watermark(&conn, is_new_database)
                .expect("initialize watermark");
            assert_eq!(
                read_usage_prune_high_watermark(&conn).expect("read watermark"),
                Some(UsagePruneHighWatermark {
                    epoch: 0,
                    history_complete: expected_complete,
                })
            );
        }
    }

    #[test]
    fn watermark_is_max_monotonic_and_incomplete_history_never_upgrades() {
        let db = Database::memory().expect("memory database");
        let conn = db.conn.lock().expect("database lock");
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            [
                USAGE_PRUNE_HIGH_WATERMARK_KEY,
                r#"{"epoch":200,"history_complete":false}"#,
            ],
        )
        .expect("seed watermark");

        advance_usage_prune_high_watermark(&conn, 100).expect("ignore lower watermark");
        assert_eq!(
            read_usage_prune_high_watermark(&conn).expect("read watermark"),
            Some(UsagePruneHighWatermark {
                epoch: 200,
                history_complete: false,
            })
        );

        advance_usage_prune_high_watermark(&conn, 300).expect("advance watermark");
        assert_eq!(
            read_usage_prune_high_watermark(&conn).expect("read watermark"),
            Some(UsagePruneHighWatermark {
                epoch: 300,
                history_complete: false,
            })
        );
    }

    #[test]
    fn missing_or_malformed_values_are_not_valid_evidence() {
        let db = Database::memory().expect("memory database");
        let conn = db.conn.lock().expect("database lock");
        conn.execute(
            "DELETE FROM settings WHERE key = ?1",
            [USAGE_PRUNE_HIGH_WATERMARK_KEY],
        )
        .expect("clear watermark");
        assert_eq!(
            read_usage_prune_high_watermark(&conn).expect("read missing watermark"),
            None
        );

        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, 'not-json')",
            [USAGE_PRUNE_HIGH_WATERMARK_KEY],
        )
        .expect("seed malformed watermark");
        assert_eq!(
            read_usage_prune_high_watermark(&conn).expect("read malformed watermark"),
            None
        );

        conn.execute(
            "UPDATE settings SET value = '{\"epoch\":-1,\"history_complete\":true}'
             WHERE key = ?1",
            [USAGE_PRUNE_HIGH_WATERMARK_KEY],
        )
        .expect("seed negative watermark");
        assert_eq!(
            read_usage_prune_high_watermark(&conn).expect("read negative watermark"),
            None,
            "a negative Unix cutoff is damaged provenance, not completeness evidence"
        );

        advance_usage_prune_high_watermark(&conn, 400).expect("repair conservatively");
        assert_eq!(
            read_usage_prune_high_watermark(&conn).expect("read repaired watermark"),
            Some(UsagePruneHighWatermark {
                epoch: 400,
                history_complete: false,
            })
        );
    }

    #[test]
    fn structural_query_errors_are_not_misreported_as_missing_evidence() {
        let conn = rusqlite::Connection::open_in_memory().expect("database");
        let error = read_usage_prune_high_watermark(&conn)
            .expect_err("missing settings table must remain a structural error");
        assert!(error.to_string().contains("settings"));
    }
}
