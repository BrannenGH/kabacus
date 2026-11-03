// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The CLI and stdio MCP bridge use the same authenticated HTTP router.

use anyhow::{bail, Context};
use axum::body::{to_bytes, Body};
use axum::http::Request;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tower::ServiceExt;

const MAX_MESSAGE: usize = crate::batch::MAX_BYTES + 1024 * 1024;

pub enum Gateway {
    Embedded(axum::Router),
    Socket {
        http: reqwest::Client,
        token: Option<String>,
    },
}

impl Gateway {
    pub fn socket(path: &std::path::Path) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .unix_socket(path)
            .connect_timeout(std::time::Duration::from_secs(3))
            .build()?;
        Ok(Self::Socket {
            http,
            token: std::env::var("KABACUS_ACCESS_TOKEN").ok(),
        })
    }

    pub async fn dispatch(&self, message: Value, protocol: &str) -> anyhow::Result<Option<Value>> {
        let id = message.get("id").cloned();
        let body = serde_json::to_vec(&message)?;
        if body.len() > MAX_MESSAGE {
            bail!("the MCP message exceeds the size limit");
        }
        let (status, bytes) = match self {
            Self::Embedded(router) => {
                let request = Request::post("/mcp")
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("mcp-protocol-version", protocol)
                    .body(Body::from(body))?;
                let response = router.clone().oneshot(request).await?;
                let status = response.status().as_u16();
                let bytes = to_bytes(response.into_body(), MAX_MESSAGE).await?.to_vec();
                (status, bytes)
            }
            Self::Socket { http, token } => {
                let mut request = http
                    .post("http://localhost/mcp")
                    .header("accept", "application/json, text/event-stream")
                    .header("content-type", "application/json")
                    .header("mcp-protocol-version", protocol)
                    .body(body);
                if let Some(token) = token {
                    request = request.bearer_auth(token);
                }
                let response = request.send().await?;
                let status = response.status().as_u16();
                (status, response.bytes().await?.to_vec())
            }
        };
        let Some(id) = id else { return Ok(None) };
        let value: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            json!({
                "message": format!("the server returned HTTP {status}: {}", String::from_utf8_lossy(&bytes))
            })
        });
        if value.get("jsonrpc").is_some() {
            return Ok(Some(value));
        }
        Ok(Some(json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32000, "message": value.get("message").and_then(Value::as_str).unwrap_or("the document request failed"), "data": value }
        })))
    }

    pub async fn call(&self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        let initialized = self.dispatch(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-03-26", "capabilities": {},
                "clientInfo": { "name": "kabacus-cli", "version": env!("CARGO_PKG_VERSION") } }
        }), "2025-03-26").await?.context("initialize returned no response")?;
        if initialized.get("error").is_some() {
            return Ok(initialized);
        }
        let protocol = initialized["result"]["protocolVersion"]
            .as_str()
            .unwrap_or("2025-03-26");
        self.dispatch(
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": name, "arguments": arguments }
            }),
            protocol,
        )
        .await?
        .context("the tool returned no response")
    }

    pub async fn stdio(&self) -> anyhow::Result<()> {
        let mut input = tokio::io::BufReader::new(tokio::io::stdin());
        let mut output = tokio::io::stdout();
        let mut protocol = "2025-03-26".to_string();
        let mut line = String::new();
        loop {
            line.clear();
            if input.read_line(&mut line).await? == 0 {
                return Ok(());
            }
            if line.len() > MAX_MESSAGE {
                bail!("the MCP message exceeds the size limit");
            }
            let response = match serde_json::from_str::<Value>(&line) {
                Ok(message) => {
                    let id = message.get("id").cloned();
                    match self.dispatch(message, &protocol).await {
                        Ok(response) => response,
                        Err(error) => id.map(|id| {
                            json!({
                                "jsonrpc": "2.0", "id": id,
                                "error": { "code": -32603, "message": error.to_string() }
                            })
                        }),
                    }
                }
                Err(_) => Some(json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": { "code": -32700, "message": "Invalid JSON" }
                })),
            };
            if let Some(response) = response {
                if let Some(version) = response
                    .pointer("/result/protocolVersion")
                    .and_then(Value::as_str)
                {
                    protocol = version.to_string();
                }
                output.write_all(&serde_json::to_vec(&response)?).await?;
                output.write_all(b"\n").await?;
                output.flush().await?;
            }
        }
    }
}
