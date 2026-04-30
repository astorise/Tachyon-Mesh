use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::process::Command;

const GUEST_COMMAND_ENV: &str = "TACHYON_GUEST_AGENT_COMMAND";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuestInvocation {
    module_id: String,
    payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GuestAgentResponse {
    status: i32,
    stdout: String,
    stderr: String,
}

fn main() {
    let mut stdin = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut stdin) {
        emit(1, "", &format!("failed to read guest payload: {error}"));
        return;
    }

    let invocation = match serde_json::from_str::<GuestInvocation>(&stdin) {
        Ok(invocation) => invocation,
        Err(error) => {
            emit(1, "", &format!("failed to parse guest payload: {error}"));
            return;
        }
    };

    let Some(command) = std::env::var_os(GUEST_COMMAND_ENV) else {
        emit(
            0,
            &format!(
                "{}:{}",
                invocation.module_id,
                serde_json::to_string(&invocation.payload).unwrap_or_default()
            ),
            "",
        );
        return;
    };

    let output = if cfg!(windows) {
        Command::new("cmd").arg("/C").arg(command).output()
    } else {
        Command::new("sh").arg("-c").arg(command).output()
    };

    match output {
        Ok(output) => emit(
            output.status.code().unwrap_or(1),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Err(error) => emit(1, "", &format!("failed to execute guest command: {error}")),
    }
}

fn emit(status: i32, stdout: &str, stderr: &str) {
    let response = GuestAgentResponse {
        status,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    };
    println!(
        "{}",
        serde_json::to_string(&response).unwrap_or_else(|_| {
            "{\"status\":1,\"stdout\":\"\",\"stderr\":\"serialization failed\"}".to_owned()
        })
    );
}
