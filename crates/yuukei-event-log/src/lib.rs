use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
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
    #[error("invalid RFC 3339 timestamp: {0}")]
    InvalidTimestamp(String),
    #[error("event log export target is part of the active database: {}", .0.display())]
    ExportTargetsDatabase(PathBuf),
    #[error("event log limit is too large: {0}")]
    LimitOutOfRange(usize),
    #[error("event log lock is poisoned")]
    PoisonedLock,
}

pub type Result<T> = std::result::Result<T, EventLogError>;

#[derive(Clone)]
pub struct EventLog {
    connection: Arc<Mutex<Connection>>,
    database_path: Option<Arc<PathBuf>>,
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
        let requested_path = path.as_ref();
        let connection = Connection::open(requested_path)?;
        let database_path = match connection.path() {
            Some("") => None,
            Some(path) => absolute_path(Path::new(path)).ok(),
            None => absolute_path(requested_path).ok(),
        }
        .map(Arc::new);
        Self::from_connection(connection, database_path)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, None)
    }

    fn from_connection(
        connection: Connection,
        database_path: Option<Arc<PathBuf>>,
    ) -> Result<Self> {
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
            database_path,
        })
    }

    pub fn append(&self, record: NewEventLogRecord) -> Result<EventLogRecord> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        append_to_connection(&connection, record)
    }

    pub fn append_batch(&self, records: Vec<NewEventLogRecord>) -> Result<Vec<EventLogRecord>> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let appended = records
            .into_iter()
            .map(|record| append_to_connection(&transaction, record))
            .collect::<Result<Vec<_>>>()?;
        transaction.commit()?;
        Ok(appended)
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
        read_from_connection(&connection, &query)
    }

    pub fn read_newest(&self, query: EventLogAdminQuery) -> Result<EventLogPage> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        read_newest_from_connection(&connection, &query)
    }

    pub fn export_jsonl(
        &self,
        query: EventLogQuery,
        path: impl AsRef<Path>,
    ) -> Result<ExportSummary> {
        let path = path.as_ref();
        self.reject_database_export_target(path)?;
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
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut matching = read_from_connection(&transaction, &query)?.records;
        if !selector.ids.is_empty() {
            matching.retain(|record| selector.ids.contains(&record.id));
        }
        if let Some(limit) = selector.before_or_at_sequence {
            matching.retain(|record| record.sequence <= limit);
        }

        let sequences = matching
            .into_iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>();
        let deleted = delete_sequences(&transaction, sequences)?;
        transaction.commit()?;
        Ok(DeleteSummary { deleted })
    }

    pub fn count_delete(&self, selector: EventLogDeleteSelector) -> Result<usize> {
        let selector = prepare_delete_selector(selector)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        count_matching_delete(&connection, &selector)
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
        let selector = prepare_delete_selector(selector)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted = delete_matching_records(&transaction, &selector)?;
        append_to_connection(&transaction, audit_record)?;
        transaction.commit()?;
        Ok(DeleteSummary { deleted })
    }

    pub fn trim_to_record_limit(
        &self,
        max_records: usize,
        fraction_divisor: usize,
    ) -> Result<TrimSummary> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record_count = count_records(&transaction)?;
        if record_count <= max_records {
            transaction.commit()?;
            return Ok(TrimSummary {
                deleted: 0,
                oldest_timestamp: None,
                newest_timestamp: None,
            });
        }
        let delete_count = (max_records / fraction_divisor.max(1)).max(1);
        let (oldest_timestamp, last_sequence, newest_timestamp) =
            trim_boundaries(&transaction, delete_count)?;
        let deleted = transaction.execute(
            "DELETE FROM event_log_records WHERE sequence <= ?1",
            params![last_sequence],
        )?;
        transaction.commit()?;
        Ok(TrimSummary {
            deleted,
            oldest_timestamp: Some(oldest_timestamp),
            newest_timestamp: Some(newest_timestamp),
        })
    }

    fn delete_matching(&self, selector: EventLogDeleteSelector) -> Result<DeleteSummary> {
        let selector = prepare_delete_selector(selector)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| EventLogError::PoisonedLock)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted = delete_matching_records(&transaction, &selector)?;
        transaction.commit()?;
        Ok(DeleteSummary { deleted })
    }

    fn reject_database_export_target(&self, path: &Path) -> Result<()> {
        let Some(database_path) = self.database_path.as_deref() else {
            return Ok(());
        };
        let target = normalized_path(path)?;
        let canonical_database_path = normalized_path(database_path)?;
        for base_path in [database_path.clone(), canonical_database_path] {
            for protected_path in [
                base_path.clone(),
                path_with_suffix(&base_path, "-wal"),
                path_with_suffix(&base_path, "-shm"),
                path_with_suffix(&base_path, "-journal"),
            ] {
                if normalized_path(&protected_path)? == target {
                    return Err(EventLogError::ExportTargetsDatabase(target));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum PreparedDeleteSelector {
    BeforeTimestamp(DateTime<Utc>),
    KindPrefix(String),
    All,
}

fn append_to_connection(
    connection: &Connection,
    record: NewEventLogRecord,
) -> Result<EventLogRecord> {
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
            &record.id,
            &record.kind,
            &record.timestamp,
            &record.resident_id,
            &record.source,
            record.device_id.as_deref(),
            record.surface_id.as_deref(),
            record.actor_id.as_deref(),
            &payload,
            causality.as_deref(),
            privacy.as_deref(),
        ],
    );

    match result {
        Ok(_) => {
            let sequence = connection.last_insert_rowid();
            read_by_sequence(connection, sequence)?
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

fn read_from_connection(connection: &Connection, query: &EventLogQuery) -> Result<EventLogPage> {
    if query.limit == Some(0) {
        return Ok(EventLogPage {
            records: Vec::new(),
            next_cursor: None,
        });
    }
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
        if matches_query(&record, query) {
            records.push(record);
            if query.limit.is_some_and(|limit| records.len() >= limit) {
                break;
            }
        }
    }

    let next_cursor = records.last().map(|record| record.sequence);
    Ok(EventLogPage {
        records,
        next_cursor,
    })
}

fn read_newest_from_connection(
    connection: &Connection,
    query: &EventLogAdminQuery,
) -> Result<EventLogPage> {
    if query.limit == Some(0) {
        return Ok(EventLogPage {
            records: Vec::new(),
            next_cursor: None,
        });
    }
    let mut statement = connection.prepare(
        r#"
        SELECT sequence, id, kind, timestamp, resident_id, source, device_id, surface_id,
               actor_id, payload, causality, privacy
        FROM event_log_records
        ORDER BY sequence DESC
        "#,
    )?;

    let mut records = Vec::new();
    let mut has_more = false;
    let rows = statement.query_map([], row_to_record)?;
    for row in rows {
        let record = row?;
        if !matches_admin_query(&record, query) {
            continue;
        }
        if query.limit.is_some_and(|limit| records.len() >= limit) {
            has_more = true;
            break;
        }
        records.push(record);
    }

    let next_cursor = has_more
        .then(|| records.last().map(|record| record.sequence))
        .flatten();
    Ok(EventLogPage {
        records,
        next_cursor,
    })
}

fn prepare_delete_selector(selector: EventLogDeleteSelector) -> Result<PreparedDeleteSelector> {
    match selector {
        EventLogDeleteSelector::BeforeTimestamp(timestamp) => Ok(
            PreparedDeleteSelector::BeforeTimestamp(parse_timestamp(&timestamp)?),
        ),
        EventLogDeleteSelector::KindPrefix(prefix) => {
            Ok(PreparedDeleteSelector::KindPrefix(prefix))
        }
        EventLogDeleteSelector::All => Ok(PreparedDeleteSelector::All),
    }
}

fn parse_timestamp(timestamp: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| EventLogError::InvalidTimestamp(timestamp.to_string()))
}

fn count_matching_delete(
    connection: &Connection,
    selector: &PreparedDeleteSelector,
) -> Result<usize> {
    match selector {
        PreparedDeleteSelector::BeforeTimestamp(timestamp) => {
            Ok(sequences_before_timestamp(connection, timestamp)?.len())
        }
        PreparedDeleteSelector::KindPrefix(prefix) => count_query(
            connection,
            "SELECT COUNT(*) FROM event_log_records WHERE instr(kind, ?1) = 1",
            Some(prefix),
        ),
        PreparedDeleteSelector::All => {
            count_query(connection, "SELECT COUNT(*) FROM event_log_records", None)
        }
    }
}

fn count_query(connection: &Connection, sql: &str, parameter: Option<&str>) -> Result<usize> {
    let count: i64 = match parameter {
        Some(parameter) => connection.query_row(sql, params![parameter], |row| row.get(0))?,
        None => connection.query_row(sql, [], |row| row.get(0))?,
    };
    usize::try_from(count).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, count).into())
}

fn count_records(connection: &Connection) -> Result<usize> {
    count_query(connection, "SELECT COUNT(*) FROM event_log_records", None)
}

fn sequences_before_timestamp(connection: &Connection, cutoff: &DateTime<Utc>) -> Result<Vec<i64>> {
    let mut statement = connection
        .prepare("SELECT sequence, timestamp FROM event_log_records ORDER BY sequence ASC")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut sequences = Vec::new();
    for row in rows {
        let (sequence, timestamp) = row?;
        if parse_timestamp(&timestamp)? < *cutoff {
            sequences.push(sequence);
        }
    }
    Ok(sequences)
}

fn delete_matching_records(
    connection: &Connection,
    selector: &PreparedDeleteSelector,
) -> Result<usize> {
    match selector {
        PreparedDeleteSelector::BeforeTimestamp(timestamp) => delete_sequences(
            connection,
            sequences_before_timestamp(connection, timestamp)?,
        ),
        PreparedDeleteSelector::KindPrefix(prefix) => Ok(connection.execute(
            "DELETE FROM event_log_records WHERE instr(kind, ?1) = 1",
            params![prefix],
        )?),
        PreparedDeleteSelector::All => Ok(connection.execute("DELETE FROM event_log_records", [])?),
    }
}

fn delete_sequences(
    connection: &Connection,
    sequences: impl IntoIterator<Item = i64>,
) -> Result<usize> {
    let mut statement =
        connection.prepare_cached("DELETE FROM event_log_records WHERE sequence = ?1")?;
    let mut deleted = 0;
    for sequence in sequences {
        deleted += statement.execute(params![sequence])?;
    }
    Ok(deleted)
}

fn trim_boundaries(connection: &Connection, delete_count: usize) -> Result<(String, i64, String)> {
    let oldest_timestamp = connection.query_row(
        "SELECT timestamp FROM event_log_records ORDER BY sequence ASC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let offset = i64::try_from(delete_count.saturating_sub(1))
        .map_err(|_| EventLogError::LimitOutOfRange(delete_count))?;
    let (last_sequence, newest_timestamp) = connection.query_row(
        "SELECT sequence, timestamp FROM event_log_records ORDER BY sequence ASC LIMIT 1 OFFSET ?1",
        params![offset],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((oldest_timestamp, last_sequence, newest_timestamp))
}

fn normalized_path(path: &Path) -> std::io::Result<PathBuf> {
    if let Ok(path) = fs::canonicalize(path) {
        return Ok(path);
    }
    let absolute = absolute_path(path)?;
    let Some(parent) = absolute.parent() else {
        return Ok(absolute);
    };
    let Ok(parent) = fs::canonicalize(parent) else {
        return Ok(absolute);
    };
    Ok(absolute
        .file_name()
        .map(|file_name| parent.join(file_name))
        .unwrap_or(parent))
}

fn absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
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
    fn sqlite_memory_path_is_not_treated_as_a_database_file() -> Result<()> {
        let log = EventLog::open(":memory:")?;
        assert!(log.database_path.is_none());
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
    fn append_batch_rolls_back_all_records_when_one_fails() -> Result<()> {
        let log = EventLog::in_memory()?;
        let error = log
            .append_batch(vec![
                record("evt_duplicate", "conversation.text"),
                record("evt_duplicate", "dialogue.say"),
            ])
            .unwrap_err();

        assert!(matches!(error, EventLogError::DuplicateRecord(_)));
        assert!(log.read(EventLogQuery::default())?.records.is_empty());
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
    fn zero_limits_return_empty_pages_without_advancing_the_cursor() -> Result<()> {
        let log = EventLog::in_memory()?;
        log.append(record("evt_1", "conversation.text"))?;

        let oldest = log.read(EventLogQuery {
            limit: Some(0),
            ..Default::default()
        })?;
        assert!(oldest.records.is_empty());
        assert_eq!(oldest.next_cursor, None);

        let newest = log.read_newest(EventLogAdminQuery {
            limit: Some(0),
            ..Default::default()
        })?;
        assert!(newest.records.is_empty());
        assert_eq!(newest.next_cursor, None);
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
    fn export_rejects_the_active_database_path_without_damaging_it() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("events.sqlite3");
        let log = EventLog::open(&path)?;
        log.append(record("evt_1", "conversation.text"))?;

        let error = log
            .export_jsonl(EventLogQuery::default(), &path)
            .unwrap_err();
        assert!(matches!(error, EventLogError::ExportTargetsDatabase(_)));
        let wal_path = path_with_suffix(&path, "-wal");
        let error = log
            .export_jsonl(EventLogQuery::default(), wal_path)
            .unwrap_err();
        assert!(matches!(error, EventLogError::ExportTargetsDatabase(_)));
        assert_eq!(log.read(EventLogQuery::default())?.records.len(), 1);

        drop(log);
        let reopened = EventLog::open(path)?;
        assert_eq!(reopened.read(EventLogQuery::default())?.records.len(), 1);
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
    fn delete_before_compares_rfc3339_timestamps_as_instants() -> Result<()> {
        let log = EventLog::in_memory()?;
        let mut offset_earlier = record("evt_offset_earlier", "conversation.text");
        offset_earlier.timestamp = "2026-07-01T10:00:00+09:00".to_string();
        log.append(offset_earlier)?;
        let mut fractional_earlier = record("evt_fractional_earlier", "conversation.text");
        fractional_earlier.timestamp = "2026-07-01T02:00:00Z".to_string();
        log.append(fractional_earlier)?;
        let mut later = record("evt_later", "conversation.text");
        later.timestamp = "2026-07-01T11:30:00+09:00".to_string();
        log.append(later)?;
        let mut equal = record("evt_equal", "conversation.text");
        equal.timestamp = "2026-07-01T02:00:00.500Z".to_string();
        log.append(equal)?;

        let cutoff = "2026-07-01T02:00:00.500Z";
        assert_eq!(
            log.count_delete(EventLogDeleteSelector::BeforeTimestamp(cutoff.to_string()))?,
            2
        );
        assert_eq!(log.delete_before(cutoff)?, DeleteSummary { deleted: 2 });
        assert_eq!(
            log.read(EventLogQuery::default())?
                .records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_later", "evt_equal"]
        );
        Ok(())
    }

    #[test]
    fn invalid_delete_timestamp_is_rejected_without_deleting_records() -> Result<()> {
        let log = EventLog::in_memory()?;
        log.append(record("evt_1", "conversation.text"))?;

        let error = log.delete_before("not-a-timestamp").unwrap_err();
        assert!(matches!(error, EventLogError::InvalidTimestamp(_)));
        assert_eq!(log.read(EventLogQuery::default())?.records.len(), 1);
        Ok(())
    }

    #[test]
    fn kind_prefix_deletion_treats_sql_wildcards_as_plain_text() -> Result<()> {
        let log = EventLog::in_memory()?;
        log.append(record("evt_percent", "desktop.%literal"))?;
        log.append(record("evt_underscore", "desktop._literal"))?;
        log.append(record("evt_window", "desktop.window"))?;

        assert_eq!(
            log.delete_by_kind_prefix("desktop.%")?,
            DeleteSummary { deleted: 1 }
        );
        assert_eq!(
            log.delete_by_kind_prefix("desktop._")?,
            DeleteSummary { deleted: 1 }
        );
        assert_eq!(
            log.read(EventLogQuery::default())?.records[0].id,
            "evt_window"
        );
        Ok(())
    }

    #[test]
    fn delete_rolls_back_when_one_record_cannot_be_deleted() -> Result<()> {
        let log = EventLog::in_memory()?;
        log.append(record("evt_1", "conversation.text"))?;
        log.append(record("evt_2", "conversation.text"))?;
        {
            let connection = log
                .connection
                .lock()
                .map_err(|_| EventLogError::PoisonedLock)?;
            connection.execute_batch(
                r#"
                CREATE TRIGGER reject_evt_2_delete
                BEFORE DELETE ON event_log_records
                WHEN OLD.id = 'evt_2'
                BEGIN
                    SELECT RAISE(ABORT, 'blocked for test');
                END;
                "#,
            )?;
        }

        let error = log.delete(DeleteSelector::default()).unwrap_err();
        assert!(matches!(error, EventLogError::Sqlite(_)));
        assert_eq!(
            log.read(EventLogQuery::default())?
                .records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_1", "evt_2"]
        );
        Ok(())
    }

    #[test]
    fn delete_with_audit_rolls_back_when_the_audit_cannot_be_appended() -> Result<()> {
        let log = EventLog::in_memory()?;
        log.append(record("evt_delete", "conversation.text"))?;
        log.append(record("evt_audit", "dialogue.say"))?;

        let error = log
            .delete_with_audit(
                EventLogDeleteSelector::KindPrefix("conversation.".to_string()),
                record("evt_audit", "event_log.deleted"),
            )
            .unwrap_err();

        assert!(matches!(error, EventLogError::DuplicateRecord(_)));
        assert_eq!(
            log.read(EventLogQuery::default())?
                .records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_delete", "evt_audit"]
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
