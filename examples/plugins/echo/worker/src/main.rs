use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

fn response(input: &Value) -> Option<Value> {
    (input["type"] == "chat" && input["data"]["message"] == "!ping")
        .then(|| json!({"type":"pong","data":{"text":"pong"}}))
}

fn main() {
    let mut _config = None;
    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("input error: {error}");
                continue;
            }
        };
        let input: Value = match serde_json::from_str(&line) {
            Ok(input) => input,
            Err(error) => {
                eprintln!("JSON error: {error}");
                continue;
            }
        };
        if input["type"] == "init" {
            _config = Some(input["config"].clone());
        }
        if let Some(output) = response(&input) {
            let mut stdout = io::stdout().lock();
            if let Err(error) = writeln!(stdout, "{output}").and_then(|()| stdout.flush()) {
                eprintln!("output error: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responds_only_to_ping_chat_messages() {
        assert_eq!(
            response(&json!({"type":"chat","data":{"message":"!ping"}})),
            Some(json!({"type":"pong","data":{"text":"pong"}}))
        );
        assert_eq!(
            response(&json!({"type":"chat","data":{"message":"hola"}})),
            None
        );
        assert_eq!(response(&json!({"type":"init","config":{}})), None);
    }
}
