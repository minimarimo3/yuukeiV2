use super::*;

#[derive(Clone, Debug, Default)]
pub(crate) struct LlmDelegationCounters {
    cooldowns: BTreeMap<String, DateTime<Utc>>,
    daily_budget: Option<DailyBudgetCounter>,
}

#[derive(Clone, Debug)]
pub(crate) struct DailyBudgetCounter {
    date: NaiveDate,
    used: u32,
}

#[derive(Debug, Default)]
pub(crate) struct InterpretationState {
    pub(crate) in_flight: bool,
    pub(crate) queued_events: VecDeque<(RuntimeEvent, EventLogRecord)>,
    pub(crate) pending_choice: Option<PendingChoice>,
}

impl ResidentHome {
    pub(crate) async fn maybe_generate_dialogue_fallback(
        &self,
        event: &RuntimeEvent,
        aliases: &SignalAliasTable,
    ) -> Result<Vec<RuntimeCommand>> {
        if !runtime_event_is_extension_readable(event) {
            return Ok(Vec::new());
        }
        let Some(delegation) = self
            .world_pack
            .llm_delegation_for_signal_with_aliases(&event.kind, aliases)
        else {
            return Ok(Vec::new());
        };
        if !self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .has_healthy_provider(DIALOGUE_GENERATE_CAPABILITY)
        {
            return Ok(Vec::new());
        }
        let canonical_signal = aliases.canonicalize(&delegation.signal);
        if !self.try_start_llm_delegation(&canonical_signal, delegation.cooldown_seconds)? {
            return Ok(Vec::new());
        }

        let mut input =
            self.dialogue_generate_input(event, &self.world_pack.default_actor_id, None, None)?;
        input.memories = self.retrieve_memories_for_dialogue_generate(event).await?;
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: DIALOGUE_GENERATE_CAPABILITY.to_string(),
            method: "generate".to_string(),
            resident_id: event.resident_id.clone(),
            actor_id: Some(self.world_pack.default_actor_id.clone()),
            input: json_map_omitting_null_values(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, event, None)?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                self.runtime_settings.llm_timeout,
                event,
            )
            .await?
        else {
            return Ok(Vec::new());
        };

        self.record_capability_result(CapabilityResultRecord {
            invocation_id: result.invocation_id,
            extension_id: result.extension_id,
            capability: result.capability,
            output: result.output.clone(),
            metadata: result.metadata,
            source_event: event,
            source_command_id: None,
            actor_id: invocation.actor_id.as_deref(),
        })?;
        let output_value = Value::Object(result.output.into_iter().collect());
        let Ok(output) = serde_json::from_value::<DialogueGenerateOutput>(output_value) else {
            return Ok(Vec::new());
        };
        self.commands_from_dialogue_generate_output(output, event)
    }

    pub(crate) fn try_start_llm_delegation(
        &self,
        signal: &str,
        cooldown_seconds: Option<u64>,
    ) -> Result<bool> {
        let now = Utc::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        if let Some(limit) = self.world_pack.llm_delegation.daily_budget {
            let today = now.date_naive();
            let counter = state
                .llm_delegation
                .daily_budget
                .get_or_insert(DailyBudgetCounter {
                    date: today,
                    used: 0,
                });
            if counter.date != today {
                counter.date = today;
                counter.used = 0;
            }
            if counter.used >= limit {
                return Ok(false);
            }
        }
        if let Some(cooldown_seconds) = cooldown_seconds {
            if let Some(last_called_at) = state.llm_delegation.cooldowns.get(signal) {
                let cooldown_seconds = i64::try_from(cooldown_seconds).unwrap_or(i64::MAX);
                if now.signed_duration_since(*last_called_at).num_seconds() < cooldown_seconds {
                    return Ok(false);
                }
            }
        }
        state
            .llm_delegation
            .cooldowns
            .insert(signal.to_string(), now);
        Ok(true)
    }

    pub(crate) fn record_llm_speech_budget_use(&self) -> Result<()> {
        if self.world_pack.llm_delegation.daily_budget.is_none() {
            return Ok(());
        }
        let today = Utc::now().date_naive();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        let counter = state
            .llm_delegation
            .daily_budget
            .get_or_insert(DailyBudgetCounter {
                date: today,
                used: 0,
            });
        if counter.date != today {
            counter.date = today;
            counter.used = 0;
        }
        counter.used = counter.used.saturating_add(1);
        Ok(())
    }

    pub(crate) fn dialogue_generate_input(
        &self,
        event: &RuntimeEvent,
        actor_id: &str,
        instruction: Option<String>,
        memories: Option<Vec<String>>,
    ) -> Result<DialogueGenerateInput> {
        let actor = self
            .world_pack
            .actors
            .iter()
            .find(|actor| actor.id == actor_id)
            .ok_or_else(|| {
                ResidentHomeError::MissingRequiredCapabilities(format!(
                    "actor is not declared: {actor_id}"
                ))
            })?;
        Ok(DialogueGenerateInput {
            event: DialogueGenerateEvent {
                kind: event.kind.clone(),
                payload: event.payload.clone(),
            },
            instruction,
            memories,
            persona: DialogueGeneratePersona {
                actor_id: actor.id.clone(),
                display_name: actor.display_name.clone(),
                profile: actor.profile.clone(),
            },
            recent_context: self.recent_dialogue_context(&event.resident_id)?,
            constraints: DialogueGenerateConstraints {
                max_length: MAX_DIALOGUE_GENERATE_LENGTH,
            },
        })
    }

    pub(crate) fn recent_dialogue_context(
        &self,
        resident_id: &str,
    ) -> Result<Vec<DialogueGenerateRecentContext>> {
        let limit = self.runtime_settings.recent_context_count;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut records = self
            .event_log
            .read(EventLogQuery {
                resident_id: Some(resident_id.to_string()),
                kind: None,
                after_sequence: None,
                limit: None,
                extension_readable_only: true,
            })?
            .records;
        if records.len() > limit {
            records = records.split_off(records.len() - limit);
        }
        Ok(records
            .into_iter()
            .map(|record| DialogueGenerateRecentContext {
                kind: record.kind,
                timestamp: record.timestamp,
                payload: major_payload(record.payload),
            })
            .collect())
    }

    pub(crate) fn commands_from_dialogue_generate_output(
        &self,
        output: DialogueGenerateOutput,
        source_event: &RuntimeEvent,
    ) -> Result<Vec<RuntimeCommand>> {
        if !output.speak {
            return Ok(Vec::new());
        }
        let Some(text) = output.text.filter(|text| valid_generated_text(text)) else {
            return Ok(Vec::new());
        };
        self.record_llm_speech_budget_use()?;

        let actor_id = self.world_pack.default_actor_id.clone();
        let mut commands = Vec::new();
        if let Some(expression) = output.expression.filter(|value| !value.trim().is_empty()) {
            let mut command =
                generated_command("avatar.expression", source_event, actor_id.clone());
            command.payload = JsonMap::from([
                ("expression".to_string(), Value::String(expression)),
                ("speakerId".to_string(), Value::String(actor_id.clone())),
                (
                    "sourceCapability".to_string(),
                    Value::String(DIALOGUE_GENERATE_CAPABILITY.to_string()),
                ),
            ]);
            commands.push(command);
        }
        if let Some(motion) = output.motion.filter(|value| !value.trim().is_empty()) {
            let mut command = generated_command("avatar.motion", source_event, actor_id.clone());
            command.payload = JsonMap::from([
                ("motion".to_string(), Value::String(motion)),
                ("speakerId".to_string(), Value::String(actor_id.clone())),
                (
                    "sourceCapability".to_string(),
                    Value::String(DIALOGUE_GENERATE_CAPABILITY.to_string()),
                ),
            ]);
            commands.push(command);
        }
        let mut command = generated_command("dialogue.say", source_event, actor_id.clone());
        command.payload = JsonMap::from([
            ("text".to_string(), Value::String(text)),
            ("speakerId".to_string(), Value::String(actor_id)),
            ("emotion".to_string(), Value::String("neutral".to_string())),
            (
                "sourceCapability".to_string(),
                Value::String(DIALOGUE_GENERATE_CAPABILITY.to_string()),
            ),
        ]);
        commands.push(command);
        Ok(commands)
    }

    async fn generate_dialogue_for_daihon(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonGenerateRequest,
    ) -> Result<Option<DaihonGenerateResponse>> {
        self.set_interpretation_in_flight(true)?;
        let result = self
            .invoke_dialogue_generate_for_daihon(source_event, request)
            .await
            .unwrap_or(None);
        self.set_interpretation_in_flight(false)?;
        Ok(result)
    }

    pub(crate) async fn invoke_dialogue_generate_for_daihon(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonGenerateRequest,
    ) -> Result<Option<DaihonGenerateResponse>> {
        if !runtime_event_is_extension_readable(source_event) {
            return Ok(None);
        }
        let actor_id = request
            .speaker_id
            .as_deref()
            .unwrap_or(&self.world_pack.default_actor_id)
            .to_string();
        let mut input = self.dialogue_generate_input(
            source_event,
            &actor_id,
            Some(request.instruction.clone()),
            None,
        )?;
        input.memories = self
            .retrieve_memories_for_dialogue_generate(source_event)
            .await?;
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: DIALOGUE_GENERATE_CAPABILITY.to_string(),
            method: "generate".to_string(),
            resident_id: source_event.resident_id.clone(),
            actor_id: Some(actor_id.clone()),
            input: json_map_from_value(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, source_event, None)?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                self.runtime_settings.llm_timeout,
                source_event,
            )
            .await?
        else {
            return Ok(None);
        };

        self.record_capability_result(CapabilityResultRecord {
            invocation_id: result.invocation_id,
            extension_id: result.extension_id,
            capability: result.capability,
            output: result.output.clone(),
            metadata: result.metadata,
            source_event,
            source_command_id: None,
            actor_id: invocation.actor_id.as_deref(),
        })?;
        let output_value = Value::Object(result.output.into_iter().collect());
        let Ok(output) = serde_json::from_value::<DialogueGenerateOutput>(output_value) else {
            return Ok(None);
        };
        if !output.speak {
            return Ok(None);
        }
        let Some(text) = output.text.filter(|text| valid_generated_text(text)) else {
            return Ok(None);
        };
        Ok(Some(DaihonGenerateResponse {
            text,
            expression: output.expression,
            motion: output.motion,
        }))
    }

    async fn interpret_dialogue(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonInterpretRequest,
    ) -> Result<String> {
        self.set_interpretation_in_flight(true)?;
        let result = self
            .invoke_dialogue_interpret(source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string());
        self.set_interpretation_in_flight(false)?;
        Ok(result)
    }

    pub(crate) async fn invoke_dialogue_interpret(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonInterpretRequest,
    ) -> Result<String> {
        if !runtime_event_is_extension_readable(source_event) {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
        }
        let input = DialogueInterpretInput {
            question: request.question,
            choices: request.choices.clone(),
            input: DialogueInterpretTextInput {
                text: request.input_text,
            },
        };
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: DIALOGUE_INTERPRET_CAPABILITY.to_string(),
            method: "interpret".to_string(),
            resident_id: source_event.resident_id.clone(),
            actor_id: None,
            input: json_map_from_value(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, source_event, None)?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                self.runtime_settings.llm_timeout,
                source_event,
            )
            .await?
        else {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
        };

        self.record_capability_result(CapabilityResultRecord {
            invocation_id: result.invocation_id,
            extension_id: result.extension_id,
            capability: result.capability,
            output: result.output.clone(),
            metadata: result.metadata,
            source_event,
            source_command_id: None,
            actor_id: None,
        })?;
        let output_value = Value::Object(result.output.into_iter().collect());
        let Ok(output) = serde_json::from_value::<DialogueInterpretOutput>(output_value) else {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
        };
        let choice = output.choice.trim();
        if choice == UNKNOWN_INTERPRETATION
            || request.choices.iter().any(|candidate| candidate == choice)
        {
            Ok(choice.to_string())
        } else {
            Ok(UNKNOWN_INTERPRETATION.to_string())
        }
    }

    async fn extract_dialogue(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonExtractRequest,
    ) -> Result<String> {
        self.set_interpretation_in_flight(true)?;
        let result = self
            .invoke_dialogue_extract(source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string());
        self.set_interpretation_in_flight(false)?;
        Ok(result)
    }

    pub(crate) async fn invoke_dialogue_extract(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonExtractRequest,
    ) -> Result<String> {
        if !runtime_event_is_extension_readable(source_event) {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
        }
        let input = DialogueExtractInput {
            instruction: request.instruction,
            input: DialogueInterpretTextInput {
                text: request.input_text,
            },
        };
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: DIALOGUE_EXTRACT_CAPABILITY.to_string(),
            method: "extract".to_string(),
            resident_id: source_event.resident_id.clone(),
            actor_id: None,
            input: json_map_from_value(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, source_event, None)?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                self.runtime_settings.llm_timeout,
                source_event,
            )
            .await?
        else {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
        };

        self.record_capability_result(CapabilityResultRecord {
            invocation_id: result.invocation_id,
            extension_id: result.extension_id,
            capability: result.capability,
            output: result.output.clone(),
            metadata: result.metadata,
            source_event,
            source_command_id: None,
            actor_id: None,
        })?;
        let output_value = Value::Object(result.output.into_iter().collect());
        let Ok(output) = serde_json::from_value::<DialogueExtractOutput>(output_value) else {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
        };
        let value = output.value.trim();
        if output.found && !value.is_empty() && value.chars().count() <= 100 {
            Ok(value.to_string())
        } else {
            Ok(UNKNOWN_INTERPRETATION.to_string())
        }
    }

    pub(crate) fn set_interpretation_in_flight(&self, in_flight: bool) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        state.interpretation.in_flight = in_flight;
        Ok(())
    }
}

pub(crate) struct ResidentHomeInterpretHandler {
    pub(crate) home: ResidentHome,
    pub(crate) source_event: RuntimeEvent,
}

#[async_trait]
impl DaihonInterpretHandler for ResidentHomeInterpretHandler {
    async fn interpret(&mut self, request: DaihonInterpretRequest) -> String {
        self.home
            .interpret_dialogue(&self.source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string())
    }

    async fn flush_commands_before_choice(&mut self, commands: Vec<RuntimeCommand>) -> bool {
        for command in commands {
            if self
                .home
                .emit_command_for_event(command, &self.source_event)
                .await
                .is_err()
            {
                return false;
            }
        }
        true
    }

    async fn generate(&mut self, request: DaihonGenerateRequest) -> Option<DaihonGenerateResponse> {
        self.home
            .generate_dialogue_for_daihon(&self.source_event, request)
            .await
            .unwrap_or(None)
    }

    async fn extract(&mut self, request: DaihonExtractRequest) -> String {
        self.home
            .extract_dialogue(&self.source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string())
    }

    async fn choose(&mut self, request: DaihonChoiceRequest) -> String {
        self.home
            .choose_for_daihon(&self.source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string())
    }
}

pub(crate) fn json_map_from_value(value: Value) -> JsonMap {
    match value {
        Value::Object(map) => map.into_iter().collect(),
        other => JsonMap::from([("value".to_string(), other)]),
    }
}

pub(crate) fn json_map_omitting_null_values(value: Value) -> JsonMap {
    match value {
        Value::Object(map) => map
            .into_iter()
            .filter(|(_, value)| !value.is_null())
            .collect(),
        other => JsonMap::from([("value".to_string(), other)]),
    }
}

pub(crate) fn runtime_event_is_extension_readable(event: &RuntimeEvent) -> bool {
    event
        .privacy
        .as_ref()
        .is_none_or(|privacy| privacy.extension_readable)
}

fn valid_generated_text(text: &str) -> bool {
    !text.trim().is_empty() && text.chars().count() <= MAX_DIALOGUE_GENERATE_LENGTH
}

fn generated_command(
    kind: impl Into<String>,
    source_event: &RuntimeEvent,
    actor_id: String,
) -> RuntimeCommand {
    let mut command = RuntimeCommand::new(
        kind,
        "capability.dialogue.generate",
        source_event.resident_id.clone(),
    );
    command.causality = Some(Causality {
        source_event_id: Some(source_event.id.clone()),
        source_command_id: None,
        trace_id: source_event
            .causality
            .as_ref()
            .and_then(|causality| causality.trace_id.clone()),
    });
    command.target = Some(CommandTarget {
        device_id: source_event.device_id.clone(),
        surface_id: source_event.surface_id.clone(),
        actor_id: Some(actor_id),
    });
    command
}
