use super::*;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MoodState {
    pub(crate) last_evaluated_at: Option<DateTime<Utc>>,
    pub(crate) current: Option<MoodSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MoodSnapshot {
    pub(crate) mood: String,
    pub(crate) talk_desire: u8,
    pub(crate) topic: String,
}

pub(crate) enum TalkImpulseModeration {
    Dispatch(RuntimeEvent),
    Skip { source_event: RuntimeEvent },
}

impl ResidentHome {
    pub(crate) async fn maybe_evaluate_mood(
        &self,
        source_event: &RuntimeEvent,
    ) -> Result<Option<RuntimeEvent>> {
        if !runtime_event_is_extension_readable(source_event) {
            return Ok(None);
        }
        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        if !router.has_healthy_provider(MOOD_EVALUATE_CAPABILITY) {
            return Ok(None);
        }
        let interval_minutes =
            mood_interval_minutes(&router).unwrap_or(DEFAULT_MOOD_INTERVAL_MINUTES);
        if interval_minutes == 0 {
            return Ok(None);
        }
        let now = event_timestamp_or_now(source_event);
        let interval_minutes = i64::try_from(interval_minutes).unwrap_or(i64::MAX);
        {
            let state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            if state.mood.last_evaluated_at.is_some_and(|last| {
                now.signed_duration_since(last).num_minutes() < interval_minutes
            }) {
                return Ok(None);
            }
        }

        let input = self.mood_evaluate_input(source_event, now)?;
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: MOOD_EVALUATE_CAPABILITY.to_string(),
            method: "evaluate".to_string(),
            resident_id: source_event.resident_id.clone(),
            actor_id: Some(self.world_pack.default_actor_id.clone()),
            input: json_map_omitting_null_values(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, source_event, None)?;
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                MOOD_EVALUATE_TIMEOUT,
                source_event,
            )
            .await?
        else {
            return Ok(None);
        };
        self.record_capability_result(CapabilityResultRecord {
            invocation_id: result.invocation_id,
            extension_id: result.extension_id.clone(),
            capability: result.capability,
            output: result.output.clone(),
            metadata: result.metadata,
            source_event,
            source_command_id: None,
            actor_id: invocation.actor_id.as_deref(),
        })?;

        let output_value = Value::Object(result.output.into_iter().collect());
        let Ok(output) = serde_json::from_value::<MoodEvaluateOutput>(output_value) else {
            return Ok(None);
        };
        let snapshot = MoodSnapshot {
            mood: normalize_mood_word(&output.mood).to_string(),
            talk_desire: output.talk_desire.min(100),
            topic: output.topic.trim().to_string(),
        };
        let mood_to_save = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            state.mood.current = Some(snapshot.clone());
            state.mood.last_evaluated_at = Some(now);
            state.mood.clone()
        };
        save_mood_state(
            self.runtime_settings.mood_state_path.as_ref(),
            &mood_to_save,
        );
        if !self.extension_can_emit_mood_changed(&result.extension_id)? {
            return Ok(None);
        }
        Ok(Some(self.mood_changed_event(
            source_event,
            &result.extension_id,
            &snapshot,
        )))
    }

    pub(crate) fn mood_evaluate_input(
        &self,
        source_event: &RuntimeEvent,
        now: DateTime<Utc>,
    ) -> Result<MoodEvaluateInput> {
        let actor = self
            .world_pack
            .actors
            .iter()
            .find(|actor| actor.id == self.world_pack.default_actor_id)
            .ok_or_else(|| {
                ResidentHomeError::MissingRequiredCapabilities(format!(
                    "actor is not declared: {}",
                    self.world_pack.default_actor_id
                ))
            })?;
        Ok(MoodEvaluateInput {
            resident_id: source_event.resident_id.clone(),
            world_pack_id: self.world_pack.id.clone(),
            current_time: source_event.timestamp.clone(),
            time_period: source_event
                .payload
                .get("timePeriod")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            seconds_since_last_user_activity: self
                .seconds_since_last_user_activity(&source_event.resident_id, now)?,
            persona: DialogueGeneratePersona {
                actor_id: actor.id.clone(),
                display_name: actor.display_name.clone(),
                profile: actor.profile.clone(),
            },
            recent_context: self.recent_dialogue_context(&source_event.resident_id)?,
        })
    }

    pub(crate) fn seconds_since_last_user_activity(
        &self,
        resident_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<u64>> {
        let records = self
            .event_log
            .read(EventLogQuery {
                resident_id: Some(resident_id.to_string()),
                kind: None,
                after_sequence: None,
                limit: None,
                extension_readable_only: true,
            })?
            .records;
        for record in records.into_iter().rev() {
            if !(record.kind.starts_with("conversation.")
                || record.kind.starts_with("avatar.gesture."))
            {
                continue;
            }
            let Some(timestamp) = event_record_timestamp(&record.timestamp) else {
                continue;
            };
            return Ok(now
                .signed_duration_since(timestamp)
                .to_std()
                .ok()
                .map(|duration| duration.as_secs()));
        }
        Ok(None)
    }

    pub(crate) fn extension_can_emit_mood_changed(&self, extension_id: &str) -> Result<bool> {
        Ok(self
            .extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .can_emit_event(extension_id, MOOD_CHANGED_EVENT))
    }

    pub(crate) fn mood_changed_event(
        &self,
        source_event: &RuntimeEvent,
        extension_id: &str,
        mood: &MoodSnapshot,
    ) -> RuntimeEvent {
        let mut event = RuntimeEvent::new(
            MOOD_CHANGED_EVENT,
            "extension",
            source_event.resident_id.clone(),
        );
        event.device_id = source_event.device_id.clone();
        event.surface_id = source_event.surface_id.clone();
        event.actor_id = Some(self.world_pack.default_actor_id.clone());
        event.causality = Some(Causality {
            source_event_id: Some(source_event.id.clone()),
            source_command_id: None,
            trace_id: source_event
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        event.payload = JsonMap::from([
            ("mood".to_string(), json!(mood.mood)),
            ("talkDesire".to_string(), json!(mood.talk_desire)),
            ("topic".to_string(), json!(mood.topic)),
            (
                "yuukeiExtension".to_string(),
                json!({ "extensionId": extension_id, "hopCount": 0 }),
            ),
        ]);
        event
    }

    pub(crate) fn apply_mood_changed_event(
        &self,
        event: &RuntimeEvent,
        record: &EventLogRecord,
    ) -> Result<Option<RuntimeEvent>> {
        let snapshot = mood_snapshot_from_payload(&event.payload);
        let mood_to_save = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            state.mood.current = Some(snapshot.clone());
            state.mood.last_evaluated_at = event_record_timestamp(&record.timestamp);
            state.mood.clone()
        };
        save_mood_state(
            self.runtime_settings.mood_state_path.as_ref(),
            &mood_to_save,
        );
        if snapshot.talk_desire < self.runtime_settings.talk_desire_high {
            return Ok(None);
        }
        let mut talk = RuntimeEvent::new(
            TALK_IMPULSE_EVENT,
            "resident-home",
            event.resident_id.clone(),
        );
        talk.device_id = event.device_id.clone();
        talk.surface_id = event.surface_id.clone();
        talk.actor_id = event.actor_id.clone();
        talk.payload = current_talk_impulse_payload(&snapshot, Some("mood.changed"));
        talk.causality = Some(Causality {
            source_event_id: Some(record.id.clone()),
            source_command_id: None,
            trace_id: event
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        Ok(Some(talk))
    }

    pub(crate) fn moderate_talk_impulse_event(
        &self,
        mut event: RuntimeEvent,
    ) -> Result<TalkImpulseModeration> {
        let mood = {
            self.state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?
                .mood
                .current
                .clone()
        };
        let mood = mood.unwrap_or_else(default_mood_snapshot);
        event.payload.insert("気分".to_string(), json!(mood.mood));
        event.payload.insert("話題".to_string(), json!(mood.topic));
        event.payload.insert("mood".to_string(), json!(mood.mood));
        event.payload.insert("topic".to_string(), json!(mood.topic));
        event
            .payload
            .insert("talkDesire".to_string(), json!(mood.talk_desire));
        if mood.talk_desire < self.runtime_settings.talk_desire_low {
            return Ok(TalkImpulseModeration::Skip {
                source_event: event,
            });
        }
        Ok(TalkImpulseModeration::Dispatch(event))
    }

    pub(crate) fn record_talk_impulse_skip(&self, event: &RuntimeEvent) -> Result<()> {
        let record = NewEventLogRecord {
            id: new_id("evt"),
            kind: "presence.talk_impulse.skipped".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: event.resident_id.clone(),
            source: "resident-home".to_string(),
            device_id: event.device_id.clone(),
            surface_id: event.surface_id.clone(),
            actor_id: event.actor_id.clone(),
            payload: JsonMap::from([
                ("reason".to_string(), json!("low-talk-desire")),
                (
                    "mood".to_string(),
                    event.payload.get("mood").cloned().unwrap_or(Value::Null),
                ),
                (
                    "talkDesire".to_string(),
                    event
                        .payload
                        .get("talkDesire")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "topic".to_string(),
                    event.payload.get("topic").cloned().unwrap_or(Value::Null),
                ),
            ]),
            causality: Some(Causality {
                source_event_id: Some(event.id.clone()),
                source_command_id: None,
                trace_id: event
                    .causality
                    .as_ref()
                    .and_then(|causality| causality.trace_id.clone()),
            }),
            privacy: None,
        };
        let appended = self.event_log.append(record)?;
        self.set_cursor(appended.sequence)?;
        Ok(())
    }
}

fn mood_interval_minutes(router: &CapabilityRouter) -> Option<u64> {
    router
        .runtime_settings_for(MOOD_EVALUATE_CAPABILITY)?
        .get("mood.intervalMinutes")
        .and_then(Value::as_u64)
}

pub(crate) fn load_mood_state(path: Option<&PathBuf>) -> MoodState {
    let Some(path) = path else {
        return MoodState::default();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return MoodState::default();
    };
    let Ok(state) = serde_json::from_str::<MoodState>(&raw) else {
        return MoodState::default();
    };
    let Some(last) = state.last_evaluated_at else {
        return MoodState::default();
    };
    let age = Utc::now().signed_duration_since(last);
    if age < chrono::Duration::zero() || age > MOOD_STATE_MAX_AGE {
        return MoodState::default();
    }
    state
}

fn save_mood_state(path: Option<&PathBuf>, state: &MoodState) {
    let Some(path) = path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(state) {
        let _ = std::fs::write(path, bytes);
    }
}

fn normalize_mood_word(value: &str) -> &str {
    match value.trim() {
        "うれしい" => "うれしい",
        "たいくつ" => "たいくつ",
        "さみしい" => "さみしい",
        "心配" => "心配",
        "ねむい" => "ねむい",
        "ふつう" => "ふつう",
        _ => "ふつう",
    }
}

fn mood_snapshot_from_payload(payload: &JsonMap) -> MoodSnapshot {
    let mood = payload
        .get("mood")
        .and_then(Value::as_str)
        .map(normalize_mood_word)
        .unwrap_or("ふつう")
        .to_string();
    let talk_desire = payload
        .get("talkDesire")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value.min(100)).ok())
        .unwrap_or(50);
    let topic = payload
        .get("topic")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    MoodSnapshot {
        mood,
        talk_desire,
        topic,
    }
}

fn default_mood_snapshot() -> MoodSnapshot {
    MoodSnapshot {
        mood: "ふつう".to_string(),
        talk_desire: 50,
        topic: String::new(),
    }
}

fn current_talk_impulse_payload(mood: &MoodSnapshot, trigger: Option<&str>) -> JsonMap {
    let mut payload = JsonMap::from([
        ("気分".to_string(), json!(mood.mood)),
        ("話題".to_string(), json!(mood.topic)),
        ("mood".to_string(), json!(mood.mood)),
        ("topic".to_string(), json!(mood.topic)),
        ("talkDesire".to_string(), json!(mood.talk_desire)),
    ]);
    if let Some(trigger) = trigger {
        payload.insert("trigger".to_string(), json!(trigger));
    }
    payload
}
