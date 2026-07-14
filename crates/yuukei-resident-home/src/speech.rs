use super::*;

impl ResidentHome {
    pub(crate) fn spawn_speech_synthesis_if_needed(
        &self,
        command: RuntimeCommand,
        source_event: RuntimeEvent,
    ) -> Result<()> {
        if command.kind != "dialogue.say" {
            return Ok(());
        }
        let Some(text) = command.payload.get("text").and_then(Value::as_str) else {
            return Ok(());
        };
        if text.trim().is_empty() {
            return Ok(());
        }

        let has_provider = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .has_healthy_provider(SPEECH_SYNTHESIS_CAPABILITY);
        if !has_provider {
            return Ok(());
        }

        let home = Arc::new(self.clone());
        tokio::spawn(async move {
            let _ = home
                .synthesize_speech_for_dialogue(command, source_event)
                .await;
        });
        Ok(())
    }

    pub(crate) async fn synthesize_speech_for_dialogue(
        &self,
        command: RuntimeCommand,
        source_event: RuntimeEvent,
    ) -> Result<()> {
        let text = command
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let speaker_id = command
            .target
            .as_ref()
            .and_then(|target| target.actor_id.clone())
            .or_else(|| {
                command
                    .payload
                    .get("speakerId")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: SPEECH_SYNTHESIS_CAPABILITY.to_string(),
            method: "synthesize".to_string(),
            resident_id: command.resident_id.clone(),
            actor_id: speaker_id.clone(),
            input: JsonMap::from([
                ("text".to_string(), Value::String(text)),
                (
                    "speakerId".to_string(),
                    speaker_id.map(Value::String).unwrap_or(Value::Null),
                ),
                (
                    "emotion".to_string(),
                    command
                        .payload
                        .get("emotion")
                        .cloned()
                        .unwrap_or_else(|| Value::String("neutral".to_string())),
                ),
                (
                    "displayCommandId".to_string(),
                    Value::String(command.id.clone()),
                ),
            ]),
            context: None,
        };

        self.record_capability_request(&invocation, &source_event, Some(&command.id))?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation,
                SPEECH_SYNTHESIS_TIMEOUT,
                &source_event,
            )
            .await?
        else {
            return Ok(());
        };

        self.record_capability_result(CapabilityResultRecord {
            invocation_id: result.invocation_id,
            extension_id: result.extension_id,
            capability: result.capability,
            output: result.output.clone(),
            metadata: result.metadata,
            source_event: &source_event,
            source_command_id: Some(&command.id),
            actor_id: command
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref()),
        })?;
        let Some(audio_path) = result.output.get("audioPath").and_then(Value::as_str) else {
            return Ok(());
        };
        if audio_path.trim().is_empty() {
            return Ok(());
        }
        let mut audio_command =
            RuntimeCommand::new("audio.play", "capability", command.resident_id.clone());
        audio_command.target = command.target.clone();
        audio_command.causality = Some(Causality {
            source_event_id: command
                .causality
                .as_ref()
                .and_then(|causality| causality.source_event_id.clone())
                .or_else(|| Some(source_event.id.clone())),
            source_command_id: Some(command.id.clone()),
            trace_id: command
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        audio_command.payload = JsonMap::from([
            (
                "audioPath".to_string(),
                Value::String(audio_path.to_string()),
            ),
            (
                "durationMs".to_string(),
                result
                    .output
                    .get("durationMs")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
        ]);
        self.emit_command_for_event(audio_command, &source_event)
            .await?;
        Ok(())
    }
}
