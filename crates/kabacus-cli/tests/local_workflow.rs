// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use serde_json::{json, Value};
use std::{
    path::Path,
    process::{Command, Output},
};
fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kabacus-cli"))
        .args(args)
        .output()
        .unwrap()
}
fn json_output(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn seed(dir: &Path) -> std::path::PathBuf {
    let journal = dir.join("Household.journal");
    std::fs::write(&journal,"commodity USD1.00\naccount assets:checking\naccount expenses:groceries\naccount equity:opening\n\n2026-01-01 Opening\n    assets:checking  3218.46 USD\n    equity:opening  -3218.46 USD\n").unwrap();
    let books = dir.join("Household.kabacus");
    json_output(cli(&[
        "import",
        books.to_str().unwrap(),
        journal.to_str().unwrap(),
        "--save",
        "--json",
    ]));
    books
}
#[test]
fn scripts_preview_then_save_once_with_named_refs_and_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let books = seed(dir.path());
    let file = dir.path().join("groceries.json");
    std::fs::write(&file,json!([
  {"command":"transaction.spent","as":"groceries","params":{"date":"2026-11-03","payee":"Lunds & Byerlys","from":"assets:checking","for":"expenses:groceries","amount":"86.42"}},
  {"command":"transaction.setCleared","params":{"id":{"$ref":"groceries"}}}
 ]).to_string()).unwrap();
    let args = [
        "run",
        file.to_str().unwrap(),
        "--books",
        books.to_str().unwrap(),
    ];
    let preview = cli(&args);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert_eq!(
        String::from_utf8(preview.stdout).unwrap().lines().count(),
        2
    );
    let before = json_output(cli(&["history", books.to_str().unwrap(), "--json"]));
    assert_eq!(before.as_array().unwrap().len(), 2);
    let mut save = args.to_vec();
    save.push("--save");
    let saved = cli(&save);
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    let history = json_output(cli(&["history", books.to_str().unwrap(), "--json"]));
    assert_eq!(history.as_array().unwrap().len(), 3);
    assert_eq!(history[0]["actor"], "cli");
    std::fs::write(&file,json!([{ "command":"transaction.spent","params":{"date":"2026-11-03","payee":"Cedar Hardware","from":"assets:checking","for":"expenses:groceries","amount":"24.18"}},{"command":"transaction.spent","params":{"date":"2026-11-03","payee":"Cedar Hardware","from":"assets:checking","for":"expenses:undeclared","amount":"8.17"}}]).to_string()).unwrap();
    assert_eq!(cli(&save).status.code(), Some(1));
    let after = json_output(cli(&["history", books.to_str().unwrap(), "--json"]));
    assert_eq!(after, history);
}
#[test]
fn usage_errors_happen_before_opening_books() {
    let output = cli(&[
        "mcp",
        "--attach",
        "7979",
        "--books",
        "/missing/Household.kabacus",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(cli(&["help", "run"]).status.success());
}

async fn rpc(
    stdin: &mut tokio::process::ChildStdin,
    lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
    request: Value,
) -> Value {
    use tokio::io::AsyncWriteExt;
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let id = request.get("id").cloned();
    if id.is_none() {
        return Value::Null;
    }
    loop {
        let line = tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        if response["id"] == id.as_ref().unwrap().clone() {
            return response;
        }
    }
}
#[tokio::test]
async fn stdio_mcp_uses_the_shared_tools_and_honors_read_only() {
    use std::process::Stdio;
    use tokio::io::AsyncBufReadExt;
    let dir = tempfile::tempdir().unwrap();
    let books = seed(dir.path());
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_kabacus-cli"))
        .args(["mcp", "--books", books.to_str().unwrap(), "--read-only"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let initialized=rpc(&mut stdin,&mut lines,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"Cedar household automation","version":"1.0"}}})).await;
    assert!(initialized["result"].is_object(), "{initialized}");
    rpc(
        &mut stdin,
        &mut lines,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let list = rpc(
        &mut stdin,
        &mut lines,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    let names = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"cash_on_hand"));
    assert!(names.contains(&"get_commands"));
    assert!(!names.contains(&"add_transaction"));
    assert!(!names.contains(&"delete_transaction"));
    let version=rpc(&mut stdin,&mut lines,json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_version","arguments":{}}})).await;
    assert_eq!(version["result"]["structuredContent"]["head"], "2");
}
