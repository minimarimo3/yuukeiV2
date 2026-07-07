use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;
use yuukei_protocol::{EventLogRecord, NewEventLogRecord, Privacy};

pub const DEFAULT_MAX_EVENT_LOG_RECORDS: usize = 1_000_000;
pub const DEFAULT_EVENT_LOG_TRIM_FRACTION_DIVISOR: usize = 10;

#[derive(Debug, Error)]
pub enum EventLogError {
    #[error("event log record already exists: {0}")]
    DuplicateRecord(String),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("event log lock is poisoned")]
    PoisonedLock,
}

pub type Result<T> = std::result::Result<T, EventLogError>;

#[derive(Clone)]
pub struct EventLog {
    connection: Arc<Mutex<Connection>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EventLogQuery {
    pub resident_id: Option<String>,
    pub kind: Option<String>,
    pub after_sequence: Option<i64>,
    pub limit: Option<usize>,
    pub extension_readable_only: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EventLogAdminQuery {
    pub kind_prefix: Option<String>,
    pub privacy_category: EventLogPrivacyFilter,
    pub before_sequence: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum EventLogPrivacyFilter {
    #[default]
    All,
    Category(String),
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventLogPage {
    pub records: Vec<EventLogRecord>,
    pub next_cursor: Option<i64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeleteSelector {
    pub ids: Vec<String>,
    pub resident_id: Option<String>,
    pub kind: Option<String>,
    pub before_or_at_sequence: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteSummary {
    pub deleted: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrimSummary {
    pub deleted: usize,
    pub oldest_timestamp: Option<String>,
    pub newest_timestamp: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventLogDeleteSelector {
    BeforeTimestamp(String),
    KindPrefix(String),
    All,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportSummary {
    pub exported: usize,
}

impl EventLog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS event_log_records (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                kind TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                resident_id TEXT NOT NULL,
                source TEXT NOT NULL,
                device_id TEXT,
                surface_id TEXT,
                actor_id TEXT,
                payload TEXT NOT NULL,
                causality TEXT,
                privacy TEXT
            );
            CREATE INDEX IF NOT EXISTS event_log_resident_sequence
                ON event_log_records (resident_id, sequence);
            CREATE INDEX IF NOT EXISTS event_log_kind_sequence
                ON event_log_records (kind, sequence);
            "#,
        )?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn append(&self, record: NewEventLogRecord) -> Result<EventLogRecord> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let payload = serde_json::to_string(&record.payload)?;
        let causality = record
            .causality
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let privacy = record
            .privacy
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        let result = connection.execute(
            r#"
            INSERT INTO event_log_records (
                id, kind, timestamp, resident_id, source, device_id, surface_id,
                actor_id, payload, causality, privacy
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                record.id,
                record.kind,
                record.timestamp,
                record.resident_id,
                record.source,
                record.device_id,
                record.surface_id,
                record.actor_id,
                payload,
                causality,
                privacy,
            ],
        );

        match result {
            Ok(_) => {
                let sequence = connection.last_insert_rowid();
                read_by_sequence(&connection, sequence)?
                    .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
            }
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(EventLogError::DuplicateRecord(record.id))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn append_batch(&self, records: Vec<NewEventLogRecord>) -> Result<Vec<EventLogRecord>> {
        records
            .into_iter()
            .map(|record| self.append(record))
            .collect()
    }

    pub fn get_by_id(&self, id: &str) -> Result<Option<EventLogRecord>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let mut statement = connection.prepare(
            r#"
            SELECT sequence, id, kind, timestamp, resident_id, source, device_id, surface_id,
                   actor_id, payload, causality, privacy
            FROM event_log_records
            WHERE id = ?1
            "#,
        )?;
        statement
            .query_row(params![id], row_to_record)
            .optional()
            .map_err(Into::into)
    }

    pub fn read(&self, query: EventLogQuery) -> Result<EventLogPage> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let mut statement = connection.prepare(
            r#"
            SELECT sequence, id, kind, timestamp, resident_id, source, device_id, surface_id,
                   actor_id, payload, causality, privacy
            FROM event_log_records
            ORDER BY sequence ASC
            "#,
        )?;

        let mut records = Vec::new();
        let rows = statement.query_map([], row_to_record)?;
        for row in rows {
            let record = row?;
            if matches_query(&record, &query) {
                records.push(record);
                if let Some(limit) = query.limit {
                    if records.len() >= limit {
                        break;
                    }
                }
            }
        }

        let next_cursor = records.last().map(|record| record.sequence);
        Ok(EventLogPage {
            records,
            next_cursor,
        })
    }

    pub fn read_newest(&self, query: EventLogAdminQuery) -> Result<EventLogPage> {
        let mut records = self.read(EventLogQuery::default())?.records;
        records.retain(|record| matches_admin_query(record, &query));
        records.reverse();
        let next_cursor = if let Some(limit) = query.limit {
            if records.len() > limit {
                records
                    .get(limit.saturating_sub(1))
                    .map(|record| record.sequence)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(limit) = query.limit {
            records.truncate(limit);
        }
        Ok(EventLogPage {
            records,
            next_cursor,
        })
    }

    pub fn export_jsonl(
        &self,
        query: EventLogQuery,
        path: impl AsRef<Path>,
    ) -> Result<ExportSummary> {
        let page = self.read(query)?;
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        for record in &page.records {
            serde_json::to_writer(&mut writer, record)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(ExportSummary {
            exported: page.records.len(),
        })
    }

    pub fn delete(&self, selector: DeleteSelector) -> Result<DeleteSummary> {
        let query = EventLogQuery {
            resident_id: selector.resident_id.clone(),
            kind: selector.kind.clone(),
            after_sequence: None,
            limit: None,
            extension_readable_only: false,
        };
        let mut matching = self.read(query)?.records;
        if !selector.ids.is_empty() {
            matching.retain(|record| selector.ids.contains(&record.id));
        }
        if let Some(limit) = selector.before_or_at_sequence {
            matching.retain(|record| record.sequence <= limit);
        }

        let connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let mut deleted = 0;
        for record in matching {
            deleted += connection.execute(
                "DELETE FROM event_log_records WHERE sequence = ?1",
                params![record.sequence],
            )?;
        }
        Ok(DeleteSummary { deleted })
    }

    pub fn count_delete(&self, selector: EventLogDeleteSelector) -> Result<usize> {
        Ok(self.records_matching_delete(selector)?.len())
    }

    pub fn delete_before(&self, timestamp: impl AsRef<str>) -> Result<DeleteSummary> {
        self.delete_matching(EventLogDeleteSelector::BeforeTimestamp(
            timestamp.as_ref().to_string(),
        ))
    }

    pub fn delete_by_kind_prefix(&self, prefix: impl AsRef<str>) -> Result<DeleteSummary> {
        self.delete_matching(EventLogDeleteSelector::KindPrefix(
            prefix.as_ref().to_string(),
        ))
    }

    pub fn delete_all(&self) -> Result<DeleteSummary> {
        self.delete_matching(EventLogDeleteSelector::All)
    }

    pub fn delete_with_audit(
        &self,
        selector: EventLogDeleteSelector,
        audit_record: NewEventLogRecord,
    ) -> Result<DeleteSummary> {
        let summary = self.delete_matching(selector)?;
        self.append(audit_record)?;
        Ok(summary)
    }

    pub fn trim_to_record_limit(
        &self,
        max_records: usize,
        fraction_divisor: usize,
    ) -> Result<TrimSummary> {
        let records = self.read(EventLogQuery::default())?.records;
        if records.len() <= max_records {
            return Ok(TrimSummary {
                deleted: 0,
                oldest_timestamp: None,
                newest_timestamp: None,
            });
        }
        let delete_count = (max_records / fraction_divisor.max(1)).max(1);
        let matching = records.into_iter().take(delete_count).collect::<Vec<_>>();
        let oldest_timestamp = matching.first().map(|record| record.timestamp.clone());
        let newest_timestamp = matching.last().map(|record| record.timestamp.clone());
        let connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let mut deleted = 0;
        for record in matching {
            deleted += connection.execute(
                "DELETE FROM event_log_records WHERE sequence = ?1",
                params![record.sequence],
            )?;
        }
        Ok(TrimSummary {
            deleted,
            oldest_timestamp,
            newest_timestamp,
        })
    }

    fn delete_matching(&self, selector: EventLogDeleteSelector) -> Result<DeleteSummary> {
        let matching = self.records_matching_delete(selector)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let mut deleted = 0;
        for record in matching {
            deleted += connection.execute(
                "DELETE FROM event_log_records WHERE sequence = ?1",
                params![record.sequence],
            )?;
        }
        Ok(DeleteSummary { deleted })
    }

    fn records_matching_delete(
        &self,
        selector: EventLogDeleteSelector,
    ) -> Result<Vec<EventLogRecord>> {
        let records = self.read(EventLogQuery::default())?.records;
        Ok(records
            .into_iter()
            .filter(|record| match &selector {
                EventLogDeleteSelector::BeforeTimestamp(timestamp) => {
                    record.timestamp.as_str() < timestamp.as_str()
                }
                EventLogDeleteSelector::KindPrefix(prefix) => record.kind.starts_with(prefix),
                EventLogDeleteSelector::All => true,
            })
            .collect())
    }
}

fn read_by_sequence(connection: &Connection, sequence: i64) -> Result<Option<EventLogRecord>> {
    let mut statement = connection.prepare(
        r#"
        SELECT sequence, id, kind, timestamp, resident_id, source, device_id, surface_id,
               actor_id, payload, causality, privacy
        FROM event_log_records
        WHERE sequence = ?1
        "#,
    )?;
    statement
        .query_row(params![sequence], row_to_record)
        .optional()
        .map_err(Into::into)
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventLogRecord> {
    let payload: String = row.get(9)?;
    let causality: Option<String> = row.get(10)?;
    let privacy: Option<String> = row.get(11)?;
    Ok(EventLogRecord {
        sequence: row.get(0)?,
        id: row.get(1)?,
        kind: row.get(2)?,
        timestamp: row.get(3)?,
        resident_id: row.get(4)?,
        source: row.get(5)?,
        device_id: row.get(6)?,
        surface_id: row.get(7)?,
        actor_id: row.get(8)?,
        payload: serde_json::from_str(&payload).map_err(json_to_sqlite)?,
        causality: causality
            .map(|value| serde_json::from_str(&value).map_err(json_to_sqlite))
            .transpose()?,
        privacy: privacy
            .map(|value| serde_json::from_str(&value).map_err(json_to_sqlite))
            .transpose()?,
    })
}

fn json_to_sqlite(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn matches_query(record: &EventLogRecord, query: &EventLogQuery) -> bool {
    if query
        .resident_id
        .as_ref()
        .is_some_and(|resident_id| record.resident_id != *resident_id)
    {
        return false;
    }
    if query.kind.as_ref().is_some_and(|kind| record.kind != *kind) {
        return false;
    }
    if query
        .after_sequence
        .is_some_and(|sequence| record.sequence <= sequence)
    {
        return false;
    }
    if query.extension_readable_only && !extension_readable(record.privacy.as_ref()) {
        return false;
    }
    true
}

fn matches_admin_query(record: &EventLogRecord, query: &EventLogAdminQuery) -> bool {
    if query
        .kind_prefix
        .as_ref()
        .is_some_and(|prefix| !record.kind.starts_with(prefix))
    {
        return false;
    }
    if query
        .before_sequence
        .is_some_and(|sequence| record.sequence >= sequence)
    {
        return false;
    }
    match &query.privacy_category {
        EventLogPrivacyFilter::All => {}
        EventLogPrivacyFilter::Category(category) => {
            if record
                .privacy
                .as_ref()
                .is_none_or(|privacy| privacy.category != *category)
            {
                return false;
            }
        }
        EventLogPrivacyFilter::None => {
            if record.privacy.is_some() {
                return false;
            }
        }
    }
    true
}

fn extension_readable(privacy: Option<&Privacy>) -> bool {
    privacy
        .map(|privacy| privacy.extension_readable)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;
    use yuukei_protocol::{
        now_timestamp, Causality, JsonMap, NewEventLogRecord, Privacy, RetentionPolicy,
    };

    use super::*;

    fn record(id: &str, kind: &str) -> NewEventLogRecord {
        NewEventLogRecord {
            id: id.to_string(),
            kind: kind.to_string(),
            timestamp: now_timestamp(),
            resident_id: "resident-default".to_string(),
            source: "test".to_string(),
            device_id: None,
            surface_id: None,
            actor_id: None,
            payload: JsonMap::from([("text".to_string(), json!("hello"))]),
            causality: Some(Causality {
                source_event_id: Some("evt_source".to_string()),
                source_command_id: None,
                trace_id: Some("trace_1".to_string()),
            }),
            privacy: None,
        }
    }

    #[test]
    fn append_read_and_reopen_persists_records() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("events.sqlite3");
        let log = EventLog::open(&path)?;
        let appended = log.append(record("evt_1", "conversation.text"))?;
        assert_eq!(appended.sequence, 1);

        drop(log);
        let reopened = EventLog::open(&path)?;
        let page = reopened.read(EventLogQuery::default())?;
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].id, "evt_1");
        assert_eq!(page.records[0].payload["text"], "hello");
        Ok(())
    }

    #[test]
    fn duplicate_ids_are_rejected() -> Result<()> {
        let log = EventLog::in_memory()?;
        log.append(record("evt_1", "conversation.text"))?;
        let error = log
            .append(record("evt_1", "conversation.text"))
            .unwrap_err();
        assert!(matches!(error, EventLogError::DuplicateRecord(_)));
        Ok(())
    }

    #[test]
    fn read_filters_by_kind_resident_cursor_and_extension_readable() -> Result<()> {
        let log = EventLog::in_memory()?;
        log.append(record("evt_1", "conversation.text"))?;
        let mut private = record("evt_2", "device.secret");
        private.privacy = Some(Privacy {
            category: "device".to_string(),
            retention: RetentionPolicy::Short,
            extension_readable: false,
        });
        log.append(private)?;

        let page = log.read(EventLogQuery {
            resident_id: Some("resident-default".to_string()),
            kind: None,
            after_sequence: Some(0),
            limit: Some(10),
            extension_readable_only: true,
        })?;
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].id, "evt_1");
        Ok(())
    }

    #[test]
    fn export_and_delete_records() -> Result<()> {
        let dir = tempdir()?;
        let export_path = dir.path().join("events.jsonl");
        let log = EventLog::in_memory()?;
        log.append(record("evt_1", "conversation.text"))?;
        log.append(record("evt_2", "dialogue.say"))?;

        let summary = log.export_jsonl(EventLogQuery::default(), &export_path)?;
        assert_eq!(summary.exported, 2);
        assert!(std::fs::read_to_string(&export_path)?.contains("\"type\":\"dialogue.say\""));

        let deleted = log.delete(DeleteSelector {
            kind: Some("conversation.text".to_string()),
            ..Default::default()
        })?;
        assert_eq!(deleted.deleted, 1);
        assert_eq!(log.read(EventLogQuery::default())?.records.len(), 1);
        Ok(())
    }

    #[test]
    fn read_newest_pages_and_filters_by_kind_prefix_and_privacy() -> Result<()> {
        let log = EventLog::in_memory()?;
        log.append(record("evt_1", "conversation.text"))?;
        let mut observed = record("evt_2", "desktop.window.appeared");
        observed.privacy = Some(Privacy {
            category: "desktop-observation".to_string(),
            retention: RetentionPolicy::Short,
            extension_readable: false,
        });
        log.append(observed)?;
        log.append(record("evt_3", "desktop.download.completed"))?;

        let page = log.read_newest(EventLogAdminQuery {
            kind_prefix: Some("desktop.".to_string()),
            limit: Some(1),
            ..Default::default()
        })?;
        assert_eq!(page.records[0].id, "evt_3");
        assert_eq!(page.next_cursor, Some(3));

        let next = log.read_newest(EventLogAdminQuery {
            kind_prefix: Some("desktop.".to_string()),
            before_sequence: page.next_cursor,
            limit: Some(10),
            ..Default::default()
        })?;
        assert_eq!(next.records[0].id, "evt_2");

        let private = log.read_newest(EventLogAdminQuery {
            privacy_category: EventLogPrivacyFilter::Category("desktop-observation".to_string()),
            ..Default::default()
        })?;
        assert_eq!(private.records.len(), 1);
        assert_eq!(private.records[0].id, "evt_2");

        let none = log.read_newest(EventLogAdminQuery {
            privacy_category: EventLogPrivacyFilter::None,
            ..Default::default()
        })?;
        assert_eq!(
            none.records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_3", "evt_1"]
        );
        Ok(())
    }

    #[test]
    fn delete_before_kind_prefix_all_and_audit_keep_sequence_increasing() -> Result<()> {
        let log = EventLog::in_memory()?;
        let mut old = record("evt_old", "conversation.text");
        old.timestamp = "2026-07-01T00:00:00.000Z".to_string();
        log.append(old)?;
        let mut recent = record("evt_recent", "desktop.window.appeared");
        recent.timestamp = "2026-07-08T00:00:00.000Z".to_string();
        log.append(recent)?;
        log.append(record("evt_dialogue", "dialogue.say"))?;

        assert_eq!(
            log.count_delete(EventLogDeleteSelector::BeforeTimestamp(
                "2026-07-02T00:00:00.000Z".to_string()
            ))?,
            1
        );
        assert_eq!(
            log.delete_before("2026-07-02T00:00:00.000Z")?,
            DeleteSummary { deleted: 1 }
        );
        assert_eq!(
            log.delete_by_kind_prefix("desktop.")?,
            DeleteSummary { deleted: 1 }
        );
        let mut audit = record("evt_audit", "event_log.deleted");
        audit.payload = JsonMap::from([("deleted".to_string(), json!(1))]);
        assert_eq!(
            log.delete_with_audit(EventLogDeleteSelector::All, audit)?,
            DeleteSummary { deleted: 1 }
        );

        let appended = log.append(record("evt_after_delete", "conversation.text"))?;
        assert_eq!(appended.sequence, 5);
        let records = log.read(EventLogQuery::default())?.records;
        assert_eq!(
            records
                .iter()
                .map(|record| record.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["event_log.deleted", "conversation.text"]
        );
        Ok(())
    }

    #[test]
    fn trim_to_record_limit_removes_oldest_fraction() -> Result<()> {
        let log = EventLog::in_memory()?;
        for index in 0..12 {
            let mut event = record(&format!("evt_{index}"), "conversation.text");
            event.timestamp = format!("2026-07-{day:02}T00:00:00.000Z", day = index + 1);
            log.append(event)?;
        }

        let summary = log.trim_to_record_limit(10, 10)?;

        assert_eq!(summary.deleted, 1);
        assert_eq!(
            summary.oldest_timestamp.as_deref(),
            Some("2026-07-01T00:00:00.000Z")
        );
        assert_eq!(
            summary.newest_timestamp.as_deref(),
            Some("2026-07-01T00:00:00.000Z")
        );
        let records = log.read(EventLogQuery::default())?.records;
        assert_eq!(records.len(), 11);
        assert_eq!(records[0].id, "evt_1");
        assert_eq!(records.last().map(|record| record.sequence), Some(12));
        Ok(())
    }
}
