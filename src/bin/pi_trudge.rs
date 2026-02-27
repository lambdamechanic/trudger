use std::env;
use std::process::{Command, ExitCode};

const DEFAULT_PROMPT_ENV: &str = "TRUDGER_AGENT_PROMPT";

fn parse_args() -> Result<(String, Vec<String>), String> {
    let mut prompt_env = DEFAULT_PROMPT_ENV.to_string();
    let args: Vec<String> = env::args().skip(1).collect();
    let mut passthrough = Vec::new();
    let mut index = 0;

    while index < args.len() {
        if args[index] == "--prompt-env" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--prompt-env requires a value".to_string())?;
            prompt_env = value.clone();
            index += 2;
            continue;
        }

        passthrough.push(args[index].clone());
        index += 1;
    }

    Ok((prompt_env, passthrough))
}

fn run() -> Result<i32, String> {
    let (prompt_env, passthrough) = parse_args()?;

    let prompt = env::var(&prompt_env).unwrap_or_default();
    let status = Command::new("pi")
        .arg("--prompt")
        .arg(prompt)
        .args(&passthrough)
        .status()
        .map_err(|err| format!("failed to launch pi command: {err}"))?;

    Ok(status.code().unwrap_or(1))
}

fn main() -> ExitCode {
    std::process::exit(match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}
