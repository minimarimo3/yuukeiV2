mod intelligence;
mod litert;
mod memory;
mod output;
mod prompt;

use std::io::{self, BufRead, Write};

use intelligence::IntelligenceRuntime;
use serde_json::{json, Value};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut runtime = IntelligenceRuntime::new();

    for line in stdin.lock().lines() {
        let result = match line {
            Ok(line) if !line.trim().is_empty() => match serde_json::from_str::<Value>(&line) {
                Ok(invocation) => runtime.dispatch(&invocation),
                Err(error) => invalid_invocation_result(error.to_string()),
            },
            Ok(_) => continue,
            Err(error) => {
                eprintln!("yuukei-intelligence: stdin failed: {error}");
                break;
            }
        };

        match serde_json::to_writer(&mut stdout, &result) {
            Ok(()) => {
                if stdout.write_all(b"\n").is_err() || stdout.flush().is_err() {
                    break;
                }
            }
            Err(error) => {
                eprintln!("yuukei-intelligence: result serialization failed: {error}");
                break;
            }
        }
    }
}

fn invalid_invocation_result(reason: String) -> Value {
    json!({
        "invocationId": "",
        "extensionId": "yuukei-intelligence",
        "capability": "dialogue.generate",
        "output": { "speak": false },
        "metadata": {
            "provider": "litert-lm",
            "model": "gemma-4-E4B-it",
            "reason": "invalid-invocation",
            "detail": reason
        }
    })
}
