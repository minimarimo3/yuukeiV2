use anyhow::anyhow;
use serde_json::{json, Value};

use crate::{litert::LiteRtEngine, memory, output, prompt};

enum EngineState {
    Uninitialized,
    Ready(LiteRtEngine),
    Failed(String),
}

pub struct IntelligenceRuntime {
    engine: EngineState,
}

impl IntelligenceRuntime {
    pub fn new() -> Self {
        Self {
            engine: EngineState::Uninitialized,
        }
    }

    pub fn dispatch(&mut self, invocation: &Value) -> Value {
        let input = invocation.get("input").unwrap_or(&Value::Null);
        let (output, metadata) = match invocation
            .get("capability")
            .and_then(Value::as_str)
            .unwrap_or("")
        {
            "dialogue.generate" => self.generate_dialogue(input),
            "dialogue.interpret" => self.interpret(input),
            "dialogue.extract" => self.extract(input),
            "memory.index" => self.index_memory(input),
            "memory.list" => memory::list(input),
            "memory.retrieve" => memory::retrieve(input),
            "memory.update" => memory::update(input),
            "memory.forget" => memory::forget(input),
            "mood.evaluate" => self.evaluate_mood(input),
            capability => (
                output::silent(),
                json!({
                    "provider": "litert-lm",
                    "model": "gemma-4-E4B-it",
                    "reason": "unsupported-capability",
                    "capability": capability
                }),
            ),
        };
        output::capability_result(invocation, output, metadata)
    }

    fn generate_dialogue(&mut self, input: &Value) -> (Value, Value) {
        let Some((system, prompt, max_tokens)) = prompt::dialogue(input) else {
            return (
                output::silent(),
                json!({
                    "provider": "litert-lm",
                    "model": "gemma-4-E4B-it",
                    "reason": "missing-daihon-instruction"
                }),
            );
        };
        let max_length = input
            .pointer("/constraints/maxLength")
            .and_then(Value::as_u64)
            .unwrap_or(120) as usize;
        match self.infer(&system, &prompt, max_tokens) {
            Ok((text, metadata)) => (output::parse_dialogue(&text, max_length), metadata),
            Err((error, metadata)) => {
                eprintln!("yuukei-intelligence: dialogue generation failed: {error:#}");
                (output::silent(), with_reason(metadata, "inference-error"))
            }
        }
    }

    fn interpret(&mut self, input: &Value) -> (Value, Value) {
        let (system, prompt, max_tokens) = prompt::interpret(input);
        let choices = input
            .get("choices")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        match self.infer(&system, &prompt, max_tokens) {
            Ok((text, metadata)) => (output::parse_interpret(&text, choices), metadata),
            Err((error, metadata)) => {
                eprintln!("yuukei-intelligence: interpretation failed: {error:#}");
                (
                    output::unknown_choice(),
                    with_reason(metadata, "inference-error"),
                )
            }
        }
    }

    fn extract(&mut self, input: &Value) -> (Value, Value) {
        let (system, prompt, max_tokens) = prompt::extract(input);
        match self.infer(&system, &prompt, max_tokens) {
            Ok((text, metadata)) => (output::parse_extract(&text), metadata),
            Err((error, metadata)) => {
                eprintln!("yuukei-intelligence: extraction failed: {error:#}");
                (
                    output::unknown_extract(),
                    with_reason(metadata, "inference-error"),
                )
            }
        }
    }

    fn index_memory(&mut self, input: &Value) -> (Value, Value) {
        if input
            .get("events")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
        {
            return memory::index(input, None, json!({}));
        }
        let (system, prompt, max_tokens) = prompt::memory_index(input);
        match self.infer(&system, &prompt, max_tokens) {
            Ok((text, metadata)) => {
                let summary = output::parse_memory_summary(&text);
                memory::index(input, summary, metadata)
            }
            Err((error, metadata)) => {
                eprintln!("yuukei-intelligence: memory summary failed: {error:#}");
                memory::index(input, None, with_reason(metadata, "inference-error"))
            }
        }
    }

    fn evaluate_mood(&mut self, input: &Value) -> (Value, Value) {
        let (system, prompt, max_tokens) = prompt::mood(input);
        match self.infer(&system, &prompt, max_tokens) {
            Ok((text, metadata)) => (output::parse_mood(&text), metadata),
            Err((error, metadata)) => {
                eprintln!("yuukei-intelligence: mood evaluation failed: {error:#}");
                (
                    output::mood_failure(),
                    with_reason(metadata, "inference-error"),
                )
            }
        }
    }

    fn infer(
        &mut self,
        system_prompt: &str,
        prompt: &str,
        max_output_tokens: i32,
    ) -> std::result::Result<(String, Value), (anyhow::Error, Value)> {
        if matches!(self.engine, EngineState::Uninitialized) {
            self.engine = match LiteRtEngine::load_packaged() {
                Ok(engine) => EngineState::Ready(engine),
                Err(error) => EngineState::Failed(format!("{error:#}")),
            };
        }
        match &self.engine {
            EngineState::Ready(engine) => {
                let metadata = engine.metadata();
                engine
                    .generate(system_prompt, prompt, max_output_tokens)
                    .map(|text| (text, metadata.clone()))
                    .map_err(|error| (error, metadata))
            }
            EngineState::Failed(message) => Err((
                anyhow!(message.clone()),
                json!({
                    "provider": "litert-lm",
                    "model": "gemma-4-E4B-it",
                    "reason": "engine-unavailable"
                }),
            )),
            EngineState::Uninitialized => unreachable!("engine state was initialized above"),
        }
    }
}

fn with_reason(mut metadata: Value, reason: &str) -> Value {
    if !metadata.is_object() {
        metadata = json!({});
    }
    if let Some(metadata) = metadata.as_object_mut() {
        metadata.insert("reason".to_string(), json!(reason));
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_uses_the_native_persistent_runtime_without_provider_settings() {
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("manifest.json"),
            )
            .expect("manifest"),
        )
        .expect("valid manifest");

        assert_eq!(
            manifest.pointer("/process/command"),
            Some(&json!("bin/yuukei-intelligence.exe"))
        );
        assert_eq!(
            manifest.pointer("/process/mode"),
            Some(&json!("persistentJsonl"))
        );
        assert!(manifest.get("config").is_none());
        assert!(manifest.get("settings").is_none());
    }

    #[test]
    fn unsupported_capability_is_safe() {
        let mut runtime = IntelligenceRuntime::new();
        let result = runtime.dispatch(&json!({
            "id": "inv_test",
            "capability": "unknown",
            "input": {}
        }));
        assert_eq!(result["output"], output::silent());
        assert_eq!(result["metadata"]["reason"], "unsupported-capability");
    }

    #[test]
    fn generic_generation_does_not_load_the_model() {
        let mut runtime = IntelligenceRuntime::new();
        let result = runtime.dispatch(&json!({
            "id": "inv_test",
            "capability": "dialogue.generate",
            "input": {}
        }));
        assert_eq!(result["output"], output::silent());
        assert_eq!(result["metadata"]["reason"], "missing-daihon-instruction");
        assert!(matches!(runtime.engine, EngineState::Uninitialized));
    }
}
