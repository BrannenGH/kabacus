// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The bundled CLI creates synthetic books and speaks stdio MCP without a daemon.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn headless_file_cli_and_stdio_mcp_share_the_gateway() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("household.kabacus");
    let binary = env!("CARGO_BIN_EXE_kabacus-cli");
    let created = Command::new(binary)
        .arg("--file")
        .arg(&file)
        .args(["create", "--name", "Household"])
        .output()
        .unwrap();
    assert!(created.status.success(), "{:?}", created.stderr);
    let duplicate = Command::new(binary)
        .arg("--file")
        .arg(&file)
        .arg("create")
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    let original = std::fs::read(&file).unwrap();
    let mut child = Command::new(binary)
        .arg("--file")
        .arg(&file)
        .arg("--recovery-dir")
        .arg(dir.path())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for message in [
        json!({
            "jsonrpc": "2.0", "id": 10, "method": "initialize",
            "params": { "protocolVersion": "2025-03-26", "capabilities": {},
                "clientInfo": { "name": "household-assistant", "version": "1.0" } }
        }),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 11, "method": "tools/list", "params": {}}),
    ] {
        writeln!(input, "{message}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let messages: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["id"], 10);
    assert!(messages[0].get("error").is_none(), "{messages:?}");
    assert_eq!(messages[1]["id"], 11);
    let tools = messages[1]["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "get_version"));
    assert!(!tools
        .iter()
        .any(|tool| tool["name"] == "delete_transaction"));
    assert_eq!(std::fs::read(file).unwrap(), original);
}
