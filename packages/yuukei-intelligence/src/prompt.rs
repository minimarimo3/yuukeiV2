use serde_json::{json, Value};

pub fn dialogue(input: &Value) -> Option<(String, String, i32)> {
    let instruction = input
        .get("instruction")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if instruction.is_empty() {
        return None;
    }
    let max_length = input
        .pointer("/constraints/maxLength")
        .and_then(Value::as_u64)
        .unwrap_or(120)
        .max(1);
    let persona = input.get("persona").cloned().unwrap_or_else(|| json!({}));
    let event = input.get("event").cloned().unwrap_or_else(|| json!({}));
    let recent_context = last_items(input.get("recentContext"), 20);
    let memories = input
        .get("memories")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|memory| !memory.is_empty())
        .take(15)
        .map(|memory| format!("- {memory}"))
        .collect::<Vec<_>>();
    let memory_section = if memories.is_empty() {
        String::new()
    } else {
        format!("\n\n覚えていること:\n{}", memories.join("\n"))
    };
    let prompt = format!(
        "You are generating exactly one in-character micro reaction for Yuukei.\n\
         Yuukei is a UI resident, not a generic assistant. The OS UI is their living space.\n\
         Daihon authored scenes always have priority. Follow the Daihon instruction below and do not continue the scene structure.\n\
         Silence is valid. If the instructed reaction would feel forced, return {{\"speak\":false}}.\n\
         If speaking, keep text at or below {max_length} characters.\n\
         Return JSON only: {{\"speak\":boolean,\"text\"?:string}}.\n\
         Do not choose an expression or motion. Default to Japanese unless the scene clearly requires another language.\n\n\
         Persona:\n{}\n\nCurrent event:\n{}\n\n\
         Daihon author instruction for this scene line:\n{instruction}\
         {memory_section}\n\nRecent context:\n{}",
        pretty(&persona),
        pretty(&event),
        pretty(&recent_context)
    );
    Some((
        "You are a dialogue.generate provider for Yuukei. Return only valid JSON. Never explain it or include Markdown.".to_string(),
        prompt,
        160,
    ))
}

pub fn interpret(input: &Value) -> (String, String, i32) {
    let question = input.get("question").and_then(Value::as_str).unwrap_or("");
    let choices = input
        .get("choices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let text = input
        .pointer("/input/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    (
        "You are a dialogue.interpret provider for Yuukei. Return only valid JSON. The choice must be listed or 不明.".to_string(),
        format!(
            "Classify the user's text for this Yuukei Daihon scene.\n\
             Choose exactly one provided value. If no choice clearly matches, choose 不明.\n\
             Do not write dialogue or add personality. Return JSON only: {{\"choice\":\"...\"}}.\n\n\
             Question:\n{question}\n\nChoices:\n{}\n\nText to classify:\n{text}",
            pretty(&Value::Array(
                choices
                    .into_iter()
                    .chain([json!("不明")])
                    .collect::<Vec<_>>()
            ))
        ),
        80,
    )
}

pub fn extract(input: &Value) -> (String, String, i32) {
    let instruction = input
        .get("instruction")
        .and_then(Value::as_str)
        .unwrap_or("");
    let text = input
        .pointer("/input/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    (
        "You are a dialogue.extract provider for Yuukei. Return only valid JSON. Never explain it."
            .to_string(),
        format!(
            "Extract one requested string value from the user's text for a Yuukei Daihon scene.\n\
             If absent, ambiguous, empty, unsupported, or longer than 100 characters, return \
             {{\"found\":false,\"value\":\"不明\"}}.\n\
             Do not write dialogue. Return JSON only: {{\"found\":boolean,\"value\":\"...\"}}.\n\n\
             Extraction instruction:\n{instruction}\n\nText to extract from:\n{text}"
        ),
        120,
    )
}

pub fn memory_index(input: &Value) -> (String, String, i32) {
    let date = input.get("date").and_then(Value::as_str).unwrap_or("");
    let digest = input
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|event| {
            let timestamp = event.get("timestamp").and_then(Value::as_str).unwrap_or("");
            let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
            let payload = event
                .get("payload")
                .and_then(Value::as_object)
                .map(|payload| {
                    payload
                        .iter()
                        .filter(|(_, value)| {
                            value.is_null()
                                || value.is_boolean()
                                || value.is_number()
                                || value.is_string()
                        })
                        .map(|(key, value)| match value.as_str() {
                            Some(value) => format!("{key}={value}"),
                            None => format!("{key}={value}"),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            format!(
                "- {timestamp} {kind}{}",
                if payload.is_empty() {
                    String::new()
                } else {
                    format!(" ({payload})")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    (
        "You are a memory.index provider for Yuukei. Return only valid JSON and do not invent facts.".to_string(),
        format!(
            "Consolidate one day of Yuukei event log records into memory notes.\n\
             Produce diary as a third-person memo in 2 to 4 sentences and 0 to 5 durable newFacts \
             such as user preferences, habits, promises, or recurring context.\n\
             Return JSON only: {{\"diary\":\"...\",\"newFacts\":[\"...\"]}}.\n\n\
             Date:\n{date}\n\nDigest lines:\n{digest}"
        ),
        320,
    )
}

pub fn mood(input: &Value) -> (String, String, i32) {
    let persona = input.get("persona").cloned().unwrap_or_else(|| json!({}));
    let name = persona
        .get("displayName")
        .or_else(|| persona.get("actorId"))
        .and_then(Value::as_str)
        .unwrap_or("Yuukei");
    let context = json!({
        "currentTime": input.get("currentTime"),
        "timePeriod": input.get("timePeriod"),
        "secondsSinceLastUserActivity": input.get("secondsSinceLastUserActivity")
    });
    (
        "You are a mood.evaluate provider for Yuukei. Return only valid JSON. Never generate dialogue.".to_string(),
        format!(
            "あなたは{name}です。最近の出来事から今の気分を評価してください。\n\
             これは発話生成ではありません。\n\
             moodは必ず ふつう, うれしい, たいくつ, さみしい, 心配, ねむい のどれか。\n\
             talkDesireは今ひとりごとを言いたい度合いを0から100の整数で。\n\
             topicは話したいことがあれば短く、なければ空文字で。\n\
             Return JSON only: {{\"mood\":\"ふつう\",\"talkDesire\":50,\"topic\":\"\"}}.\n\n\
             Persona:\n{}\n\nCurrent context:\n{}\n\nRecent context:\n{}",
            pretty(&persona),
            pretty(&context),
            pretty(&last_items(input.get("recentContext"), 20))
        ),
        120,
    )
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn last_items(value: Option<&Value>, limit: usize) -> Value {
    let Some(items) = value.and_then(Value::as_array) else {
        return json!([]);
    };
    Value::Array(items[items.len().saturating_sub(limit)..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_generation_without_daihon_instruction_is_refused() {
        assert!(dialogue(&json!({ "event": { "kind": "conversation.text" } })).is_none());
    }

    #[test]
    fn instructed_generation_contains_author_direction() {
        let (_, prompt, _) =
            dialogue(&json!({ "instruction": "少し驚いて聞き返す" })).expect("prompt");
        assert!(prompt.contains("少し驚いて聞き返す"));
    }
}
