use super::*;

impl ResidentHome {
    pub fn trim_event_log_to_record_limit(
        &self,
        max_records: usize,
        fraction_divisor: usize,
    ) -> Result<TrimSummary> {
        let summary = self
            .event_log
            .trim_to_record_limit(max_records, fraction_divisor)?;
        if summary.deleted == 0 {
            return Ok(summary);
        }
        let record = NewEventLogRecord {
            id: new_id("evt"),
            kind: "event_log.trimmed".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: self.resident_id()?,
            source: "resident-home".to_string(),
            device_id: None,
            surface_id: None,
            actor_id: None,
            payload: JsonMap::from([
                ("deleted".to_string(), json!(summary.deleted)),
                (
                    "oldestTimestamp".to_string(),
                    summary
                        .oldest_timestamp
                        .as_ref()
                        .map(|value| Value::String(value.clone()))
                        .unwrap_or(Value::Null),
                ),
                (
                    "newestTimestamp".to_string(),
                    summary
                        .newest_timestamp
                        .as_ref()
                        .map(|value| Value::String(value.clone()))
                        .unwrap_or(Value::Null),
                ),
            ]),
            causality: None,
            privacy: None,
        };
        let appended = self.event_log.append(record)?;
        self.set_cursor(appended.sequence)?;
        Ok(summary)
    }

    pub fn read_event_log_for_extension(&self, grant: EventLogReadGrant) -> Result<EventLogPage> {
        let validated = self.validate_event_log_read_grant(&grant)?;
        let page = self.event_log.read(EventLogQuery {
            resident_id: Some(grant.resident_id.clone()),
            kind: None,
            after_sequence: grant.cursor_after_sequence,
            limit: None,
            extension_readable_only: true,
        })?;

        if validated.max_records == 0 {
            return Ok(EventLogPage {
                records: Vec::new(),
                next_cursor: None,
            });
        }
        let mut records = Vec::new();
        for mut record in page.records {
            if !event_type_matches(&validated.event_types, &record.kind) {
                continue;
            }
            if let Some(until_timestamp) = validated.until_timestamp.as_ref() {
                let record_timestamp = parse_rfc3339_utc(&record.timestamp)?;
                if &record_timestamp > until_timestamp {
                    continue;
                }
            }
            if let Some(privacy) = &record.privacy {
                if !validated.privacy_categories.contains(&privacy.category) {
                    continue;
                }
            }
            if !validated.allow_payloads {
                record.payload.clear();
            } else if !validated.allow_references {
                strip_references_from_payload(&mut record.payload);
            }
            records.push(record);
            if records.len() >= validated.max_records {
                break;
            }
        }
        let next_cursor = records.last().map(|record| record.sequence);
        Ok(EventLogPage {
            records,
            next_cursor,
        })
    }

    pub fn read_event_log_page(
        &self,
        options: ResidentEventLogReadOptions,
    ) -> Result<ResidentEventLogPage> {
        let query = EventLogAdminQuery {
            kind_prefix: options
                .kind_prefix
                .filter(|prefix| !prefix.trim().is_empty()),
            privacy_category: options.privacy_category,
            before_sequence: options.before_sequence,
            limit: options.limit,
        };
        let total = self
            .event_log
            .read_newest(EventLogAdminQuery {
                limit: None,
                ..query.clone()
            })?
            .records
            .len();
        let page = self.event_log.read_newest(query)?;
        Ok(ResidentEventLogPage {
            records: page.records,
            next_cursor: page.next_cursor,
            total,
        })
    }

    pub fn count_event_log_delete_before(&self, timestamp: impl Into<String>) -> Result<usize> {
        self.event_log
            .count_delete(EventLogDeleteSelector::BeforeTimestamp(timestamp.into()))
            .map_err(Into::into)
    }

    pub fn count_event_log_delete_by_kind_prefix(
        &self,
        prefix: impl Into<String>,
    ) -> Result<usize> {
        self.event_log
            .count_delete(EventLogDeleteSelector::KindPrefix(prefix.into()))
            .map_err(Into::into)
    }

    pub fn count_event_log_delete_all(&self) -> Result<usize> {
        self.event_log
            .count_delete(EventLogDeleteSelector::All)
            .map_err(Into::into)
    }

    pub fn delete_event_log_before(&self, timestamp: impl Into<String>) -> Result<DeleteSummary> {
        let timestamp = timestamp.into();
        self.delete_event_log_with_audit(
            EventLogDeleteSelector::BeforeTimestamp(timestamp.clone()),
            json!({ "condition": "before", "timestamp": timestamp }),
        )
    }

    pub fn delete_event_log_by_kind_prefix(
        &self,
        prefix: impl Into<String>,
    ) -> Result<DeleteSummary> {
        let prefix = prefix.into();
        self.delete_event_log_with_audit(
            EventLogDeleteSelector::KindPrefix(prefix.clone()),
            json!({ "condition": "kindPrefix", "kindPrefix": prefix }),
        )
    }

    pub fn delete_event_log_all(&self) -> Result<DeleteSummary> {
        self.delete_event_log_with_audit(EventLogDeleteSelector::All, json!({ "condition": "all" }))
    }

    fn delete_event_log_with_audit(
        &self,
        selector: EventLogDeleteSelector,
        mut payload: Value,
    ) -> Result<DeleteSummary> {
        let resident_id = self.resident_id()?;
        let deleted = self.event_log.count_delete(selector.clone())?;
        if let Value::Object(map) = &mut payload {
            map.insert("deleted".to_string(), Value::Number(deleted.into()));
        }
        let audit = NewEventLogRecord {
            id: new_id("evt"),
            kind: "event_log.deleted".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id,
            source: "resident-home".to_string(),
            device_id: None,
            surface_id: None,
            actor_id: None,
            payload: json_map_from_value(payload),
            causality: None,
            privacy: None,
        };
        let summary = self.event_log.delete_with_audit(selector, audit)?;
        let page = self.event_log.read_newest(EventLogAdminQuery {
            limit: Some(1),
            ..Default::default()
        })?;
        if let Some(record) = page.records.first() {
            self.set_cursor(record.sequence)?;
        }
        Ok(summary)
    }

    fn validate_event_log_read_grant(
        &self,
        grant: &EventLogReadGrant,
    ) -> Result<ValidatedEventLogReadGrant> {
        if grant.resident_id != self.resident_id()? {
            return Err(ResidentHomeError::EventLogReadDenied(format!(
                "grant resident {} does not match this Resident Home",
                grant.resident_id
            )));
        }

        let permission = self
            .extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .event_log_read_permission(&grant.extension_id)
            .ok_or_else(|| {
                ResidentHomeError::EventLogReadDenied(format!(
                    "extension is not registered, enabled, or allowed to read the event log: {}",
                    grant.extension_id
                ))
            })?;

        let expires_at = parse_rfc3339_utc(&grant.expires_at)?;
        if expires_at <= Utc::now() {
            return Err(ResidentHomeError::EventLogReadDenied(format!(
                "grant expired at {}",
                grant.expires_at
            )));
        }
        if grant.purpose.trim().is_empty() || grant.purpose != permission.purpose {
            return Err(ResidentHomeError::EventLogReadDenied(format!(
                "grant purpose does not match manifest permission: {}",
                grant.extension_id
            )));
        }

        if permission.event_types.is_empty() {
            return Err(ResidentHomeError::EventLogReadDenied(format!(
                "extension manifest does not allow event log event types: {}",
                grant.extension_id
            )));
        }
        let event_types = if grant.event_types.is_empty() {
            permission.event_types.clone()
        } else {
            for requested in &grant.event_types {
                if !event_type_matches(&permission.event_types, requested) {
                    return Err(ResidentHomeError::EventLogReadDenied(format!(
                        "requested event type is outside manifest permission: {requested}"
                    )));
                }
            }
            grant.event_types.clone()
        };

        for requested_category in &grant.privacy_categories {
            if !permission
                .privacy_categories
                .iter()
                .any(|allowed| allowed == requested_category)
            {
                return Err(ResidentHomeError::EventLogReadDenied(format!(
                    "requested privacy category is outside manifest permission: {requested_category}"
                )));
            }
        }

        let until_timestamp = grant
            .until_timestamp
            .as_deref()
            .map(parse_rfc3339_utc)
            .transpose()?;

        Ok(ValidatedEventLogReadGrant {
            event_types,
            privacy_categories: grant.privacy_categories.clone(),
            until_timestamp,
            max_records: grant.max_records.min(permission.max_records),
            allow_payloads: grant.allow_payloads && permission.allow_payloads,
            allow_references: grant.allow_references && permission.allow_references,
        })
    }
}

#[derive(Clone, Debug)]
struct ValidatedEventLogReadGrant {
    event_types: Vec<String>,
    privacy_categories: Vec<String>,
    until_timestamp: Option<DateTime<Utc>>,
    max_records: usize,
    allow_payloads: bool,
    allow_references: bool,
}

fn parse_rfc3339_utc(timestamp: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| {
            ResidentHomeError::EventLogReadDenied(format!("invalid timestamp {timestamp}: {error}"))
        })
}

pub(crate) fn event_record_date(timestamp: &str) -> Option<NaiveDate> {
    event_record_timestamp(timestamp).map(|timestamp| timestamp.date_naive())
}

pub(crate) fn event_record_timestamp(timestamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}
