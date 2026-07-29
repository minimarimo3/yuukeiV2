use serde_json::{json, Value};

pub const RESULT_TOOL_NAME: &str = "submit_yuukei_result";

pub fn dialogue(max_length: usize) -> Value {
    json!({
        "type": "object",
        "properties": {
            "speak": { "type": "boolean" },
            "text": {
                "type": "string",
                "description": format!("発話する場合の本文。最大{}文字", max_length.max(1))
            }
        },
        "required": ["speak"],
        "additionalProperties": false
    })
}

pub fn interpret(choices: &[Value]) -> Value {
    let mut allowed = choices
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !allowed.iter().any(|choice| choice == "不明") {
        allowed.push("不明".to_string());
    }
    allowed.sort();
    allowed.dedup();

    json!({
        "type": "object",
        "properties": {
            "choice": { "type": "string", "enum": allowed }
        },
        "required": ["choice"],
        "additionalProperties": false
    })
}

pub fn extract() -> Value {
    json!({
        "type": "object",
        "properties": {
            "found": { "type": "boolean" },
            "value": { "type": "string" }
        },
        "required": ["found", "value"],
        "additionalProperties": false
    })
}

pub fn memory_index() -> Value {
    json!({
        "type": "object",
        "properties": {
            "diary": { "type": "string" },
            "newFacts": {
                "type": "array",
                "items": { "type": "string" }
            }
        },
        "required": ["diary", "newFacts"],
        "additionalProperties": false
    })
}

pub fn mood() -> Value {
    json!({
        "type": "object",
        "properties": {
            "mood": {
                "type": "string",
                "enum": ["ふつう", "うれしい", "たいくつ", "さみしい", "心配", "ねむい"]
            },
            "talkDesire": { "type": "integer" },
            "topic": { "type": "string" }
        },
        "required": ["mood", "talkDesire", "topic"],
        "additionalProperties": false
    })
}

pub fn tool_definition(schema: &Value) -> Value {
    json!([{
        "type": "function",
        "function": {
            "name": RESULT_TOOL_NAME,
            "description": "Yuukei Coreへ、このリクエストの構造化された結果を一度だけ返す",
            "parameters": schema
        }
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpret_schema_contains_only_authored_choices_and_unknown() {
        let schema = interpret(&[json!("はい"), json!("いいえ"), json!("はい")]);
        assert_eq!(
            schema.pointer("/properties/choice/enum"),
            Some(&json!(["いいえ", "はい", "不明"]))
        );
    }

    #[test]
    fn every_schema_is_closed_and_wrapped_in_one_result_tool() {
        for schema in [
            dialogue(120),
            interpret(&[json!("yes")]),
            extract(),
            memory_index(),
            mood(),
        ] {
            assert_eq!(schema["additionalProperties"], json!(false));
            let tools = tool_definition(&schema);
            assert_eq!(tools.as_array().map(Vec::len), Some(1));
            assert_eq!(
                tools.pointer("/0/function/name"),
                Some(&json!(RESULT_TOOL_NAME))
            );
            assert_eq!(tools.pointer("/0/function/parameters"), Some(&schema));
        }
    }
}
