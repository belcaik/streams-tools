use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

fn emit(input: &Value) {
    let output = json!({"type":"observed","data":{"input":input}});
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{output}").expect("output should be written");
    stdout.flush().expect("output should be flushed");
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "normal".into());
    eprintln!("test worker mode {mode}");
    if mode == "crash" {
        std::process::exit(1);
    }
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
        emit(&input);
        if input["type"] == "init" {
            if mode == "forbidden" {
                println!(r#"{{"type":"forbidden","data":{{}}}}"#);
            }
            if mode == "restart" {
                std::process::exit(1);
            }
        }
    }
}
