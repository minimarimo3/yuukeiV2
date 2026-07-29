use serde_json::{json, Map, Value};

pub fn capability_result(invocation: &Value, output: Value, metadata: Value) -> Value {
    let capability = invocation
        .get("capability")
        .and_then(Value::as_str)
        .unwrap_or("dialogue.generate");
    let normalized = match capability {
        "dialogue.interpret" => normalize_interpret(
            &output,
            invocation
                .pointer("/input/choices")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        ),
        "dialogue.extract" => normalize_extract(&output),
        "memory.index" => normalize_memory_index_capability(&output),
        "memory.list" => normalize_memory_list_capability(&output),
        "memory.retrieve" => normalize_memory_retrieve_capability(&output),
        "memory.update" => json!({ "updated": output.get("updated") == Some(&Value::Bool(true)) }),
        "memory.forget" => json!({
            "removedFacts": non_negative_integer(output.get("removedFacts")),
            "removedEpisodes": non_negative_integer(output.get("removedEpisodes"))
        }),
        "mood.evaluate" => normalize_mood(&output),
        _ => normalize_dialogue(
            &output,
            invocation
                .pointer("/input/constraints/maxLength")
                .and_then(Value::as_u64)
                .unwrap_or(120) as usize,
        ),
    };
    json!({
        "invocationId": invocation.get("id").and_then(Value::as_str).unwrap_or(""),
        "extensionId": "yuukei-intelligence",
        "capability": capability,
        "output": normalized,
        "metadata": metadata
    })
}

pub fn silent() -> Value {
    json!({ "speak": false })
}

pub fn unknown_choice() -> Value {
    json!({ "choice": "不明" })
}

pub fn unknown_extract() -> Value {
    json!({ "found": false, "value": "不明" })
}

pub fn mood_failure() -> Value {
    json!({ "mood": "ふつう", "talkDesire": 50, "topic": "" })
}

pub fn parse_dialogue(text: &str, max_length: usize) -> Value {
    parse_object(text)
        .map(|value| normalize_dialogue(&value, max_length))
        .unwrap_or_else(silent)
}

pub fn parse_interpret(text: &str, choices: &[Value]) -> Value {
    parse_object(text)
        .map(|value| normalize_interpret(&value, choices))
        .unwrap_or_else(unknown_choice)
}

pub fn parse_extract(text: &str) -> Value {
    parse_object(text)
        .map(|value| normalize_extract(&value))
        .unwrap_or_else(unknown_extract)
}

pub fn parse_memory_summary(text: &str) -> Option<Value> {
    let value = parse_object(text)?;
    let diary = value
        .get("diary")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let new_facts = value
        .get("newFacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|fact| !fact.is_empty())
        .take(5)
        .map(|fact| Value::String(fact.to_string()))
        .collect::<Vec<_>>();
    if diary.is_empty() && new_facts.is_empty() {
        return None;
    }
    Some(json!({ "diary": diary, "newFacts": new_facts }))
}

pub fn parse_mood(text: &str) -> Value {
    parse_object(text)
        .map(|value| normalize_mood(&value))
        .unwrap_or_else(mood_failure)
}

fn normalize_dialogue(value: &Value, max_length: usize) -> Value {
    if value.get("speak") != Some(&Value::Bool(true)) {
        return silent();
    }
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if text.is_empty() {
        return silent();
    }
    // Expressions and motions are intentionally not accepted from the model.
    // Daihon and the renderer remain authoritative for those asset names.
    json!({
        "speak": true,
        "text": text.chars().take(max_length.max(1)).collect::<String>()
    })
}

fn normalize_interpret(value: &Value, choices: &[Value]) -> Value {
    let choice = value
        .get("choice")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let allowed = choice == "不明"
        || choices
            .iter()
            .filter_map(Value::as_str)
            .any(|candidate| candidate == choice);
    if !allowed {
        return unknown_choice();
    }
    let mut output = Map::from_iter([("choice".to_string(), json!(choice))]);
    if let Some(confidence) = value.get("confidence").and_then(Value::as_f64) {
        if confidence.is_finite() {
            output.insert("confidence".to_string(), json!(confidence));
        }
    }
    Value::Object(output)
}

fn normalize_extract(value: &Value) -> Value {
    if value.get("found") != Some(&Value::Bool(true)) {
        return unknown_extract();
    }
    let text = value
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if text.is_empty() || text.chars().count() > 100 {
        return unknown_extract();
    }
    json!({ "found": true, "value": text })
}

fn normalize_mood(value: &Value) -> Value {
    let mood = match value.get("mood").and_then(Value::as_str).map(str::trim) {
        Some(value)
            if [
                "ふつう",
                "うれしい",
                "たいくつ",
                "さみしい",
                "心配",
                "ねむい",
            ]
            .contains(&value) =>
        {
            value
        }
        _ => "ふつう",
    };
    let talk_desire = value
        .get("talkDesire")
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .map(|number| number.trunc().clamp(0.0, 100.0) as i64)
        .unwrap_or(50);
    let topic = value
        .get("topic")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .chars()
        .take(80)
        .collect::<String>();
    json!({ "mood": mood, "talkDesire": talk_desire, "topic": topic })
}

fn normalize_memory_index_capability(value: &Value) -> Value {
    let mut output = Map::from_iter([(
        "indexed".to_string(),
        json!(value.get("indexed") == Some(&Value::Bool(true))),
    )]);
    if let Some(note_count) = value.get("noteCount") {
        output.insert(
            "noteCount".to_string(),
            json!(non_negative_integer(Some(note_count))),
        );
    }
    Value::Object(output)
}

fn normalize_memory_list_capability(value: &Value) -> Value {
    let facts = value
        .get("facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|fact| {
            let id = fact.get("id")?.as_str()?.to_string();
            let text = fact.get("text")?.as_str()?.trim().to_string();
            (!id.is_empty() && !text.is_empty()).then(|| {
                json!({
                    "id": id,
                    "text": text,
                    "createdAt": fact.get("createdAt").and_then(Value::as_str).unwrap_or(""),
                    "updatedAt": fact.get("updatedAt").and_then(Value::as_str).unwrap_or("")
                })
            })
        })
        .collect::<Vec<_>>();
    let episodes = value
        .get("episodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|episode| {
            let id = episode.get("id")?.as_str()?.to_string();
            let text = episode.get("text")?.as_str()?.trim().to_string();
            (!id.is_empty() && !text.is_empty()).then(|| {
                json!({
                    "id": id,
                    "text": text,
                    "timestamp": episode.get("timestamp").and_then(Value::as_str).unwrap_or("")
                })
            })
        })
        .collect::<Vec<_>>();
    let total = value
        .get("episodeTotal")
        .map(|value| non_negative_integer(Some(value)))
        .unwrap_or(episodes.len() as i64);
    json!({ "facts": facts, "episodes": episodes, "episodeTotal": total })
}

fn normalize_memory_retrieve_capability(value: &Value) -> Value {
    let memories = value
        .get("memories")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|memory| {
            let text = memory.get("text")?.as_str()?.trim();
            if text.is_empty() {
                return None;
            }
            let kind = if memory.get("kind").and_then(Value::as_str) == Some("episode") {
                "episode"
            } else {
                "fact"
            };
            let mut result = Map::from_iter([
                ("text".to_string(), json!(text)),
                ("kind".to_string(), json!(kind)),
            ]);
            if kind == "episode" {
                if let Some(date) = memory.get("date").and_then(Value::as_str) {
                    if !date.trim().is_empty() {
                        result.insert("date".to_string(), json!(date.trim()));
                    }
                }
            }
            Some(Value::Object(result))
        })
        .collect::<Vec<_>>();
    json!({ "memories": memories })
}

fn parse_object(text: &str) -> Option<Value> {
    serde_json::from_str::<Value>(text.trim())
        .ok()
        .filter(Value::is_object)
}

fn non_negative_integer(value: Option<&Value>) -> i64 {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .map(|number| number.trunc().max(0.0) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogue_ignores_untrusted_asset_names() {
        let output = parse_dialogue(
            r#"{"speak":true,"text":"こんにちは","expression":"unknown","motion":"unknown"}"#,
            120,
        );
        assert_eq!(output, json!({ "speak": true, "text": "こんにちは" }));
    }

    #[test]
    fn interpret_rejects_choices_outside_daihon() {
        let output = parse_interpret(r#"{"choice":"たぶん"}"#, &[json!("はい"), json!("いいえ")]);
        assert_eq!(output, unknown_choice());
    }

    #[test]
    fn rejects_json_embedded_in_surrounding_text() {
        assert_eq!(
            parse_extract("answer: {\"found\":true,\"value\":\"あんぱん\"} done"),
            unknown_extract()
        );
    }
}
