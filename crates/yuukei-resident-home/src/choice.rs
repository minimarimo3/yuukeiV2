use super::*;

#[derive(Debug)]
pub(crate) struct PendingChoice {
    choice_id: String,
    choices: Vec<String>,
    sender: oneshot::Sender<String>,
}

impl ResidentHome {
    pub(crate) fn resolve_pending_choice_event(&self, event: &RuntimeEvent) -> Result<bool> {
        if event.kind != "conversation.choice" {
            return Ok(false);
        }
        let resolved = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            let Some(pending) = state.interpretation.pending_choice.as_ref() else {
                return Ok(!state.interpretation.in_flight);
            };
            let Some(choice_id) = event.payload.get("choiceId").and_then(Value::as_str) else {
                return Ok(true);
            };
            let Some(choice) = event.payload.get("choice").and_then(Value::as_str) else {
                return Ok(true);
            };
            let Some(index) = event
                .payload
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
            else {
                return Ok(true);
            };
            let matches_pending = pending.choice_id == choice_id
                && pending.choices.get(index).map(String::as_str) == Some(choice);
            if matches_pending {
                state
                    .interpretation
                    .pending_choice
                    .take()
                    .map(|pending| (pending.sender, choice.to_string()))
            } else {
                None
            }
        };
        if let Some((sender, choice)) = resolved {
            let _ = sender.send(choice);
        }
        Ok(true)
    }

    pub(crate) async fn choose_for_daihon(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonChoiceRequest,
    ) -> Result<String> {
        let result = self
            .invoke_choice_for_daihon(source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string());
        self.set_interpretation_in_flight(false)?;
        Ok(result)
    }

    async fn invoke_choice_for_daihon(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonChoiceRequest,
    ) -> Result<String> {
        let choice_id = new_id("choice");
        let timeout_seconds = request.timeout_seconds;
        let mut command = RuntimeCommand::new(
            "dialogue.choices",
            "daihon",
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
            actor_id: Some(self.world_pack.default_actor_id.clone()),
        });
        command.payload = JsonMap::from([
            ("choiceId".to_string(), json!(choice_id)),
            ("choices".to_string(), json!(request.choices)),
            ("timeoutSeconds".to_string(), json!(timeout_seconds)),
        ]);

        let (sender, receiver) = oneshot::channel();
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            state.interpretation.pending_choice = Some(PendingChoice {
                choice_id: choice_id.clone(),
                choices: request.choices,
                sender,
            });
            state.interpretation.in_flight = true;
        }
        if let Err(error) = self.emit_command_for_event(command, source_event).await {
            self.clear_pending_choice(&choice_id)?;
            self.set_interpretation_in_flight(false)?;
            return Err(error);
        }

        match timeout(Duration::from_secs(timeout_seconds), receiver).await {
            Ok(Ok(choice)) => Ok(choice),
            Ok(Err(_)) | Err(_) => {
                self.clear_pending_choice(&choice_id)?;
                self.emit_choice_clear(source_event, &choice_id, "timeout")
                    .await?;
                Ok(UNKNOWN_INTERPRETATION.to_string())
            }
        }
    }

    fn clear_pending_choice(&self, choice_id: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        if state
            .interpretation
            .pending_choice
            .as_ref()
            .is_some_and(|pending| pending.choice_id == choice_id)
        {
            state.interpretation.pending_choice = None;
        }
        Ok(())
    }

    async fn emit_choice_clear(
        &self,
        source_event: &RuntimeEvent,
        choice_id: &str,
        reason: &str,
    ) -> Result<()> {
        let mut command = RuntimeCommand::new(
            "dialogue.choices.clear",
            "daihon",
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
            actor_id: Some(self.world_pack.default_actor_id.clone()),
        });
        command.payload = JsonMap::from([
            ("choiceId".to_string(), json!(choice_id)),
            ("reason".to_string(), json!(reason)),
        ]);
        self.emit_command_for_event(command, source_event).await?;
        Ok(())
    }
}
