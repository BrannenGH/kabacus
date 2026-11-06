// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Opt-in loopback MCP channel for a window's already open books.

use crate::{ClientError, LocalBooks};
use kabacus_store::{BookStore, Row};
use rmcp::ServiceExt;
use serde::Serialize;
use std::{
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
};

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentActivity {
    pub agent: Option<String>,
    pub changed_by: Option<String>,
    pub changed: Vec<u64>,
    pub expires_at: u128,
}
pub struct ControlChannel {
    pub port: u16,
    activity: Arc<RwLock<AgentActivity>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for ControlChannel {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl ControlChannel {
    pub fn activity(&self) -> AgentActivity {
        let mut activity = self
            .activity
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if clock() > activity.expires_at {
            activity.changed.clear();
            activity.changed_by = None;
        }
        activity
    }
    pub async fn for_client(client: crate::KabacusClient, port: u16) -> Result<Self, ClientError> {
        if let Some(books) = client.local_books().await? {
            return Self::start(books.clone(), port).await;
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(error)?;
        let port = listener.local_addr().map_err(error)?.port();
        let activity = Arc::new(RwLock::new(AgentActivity::default()));
        let updates = activity.clone();
        let task = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            while let Ok((stream, _)) = listener.accept().await {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                let mut agent = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else {
                        break;
                    };
                    if request["method"] == "initialize" {
                        agent = request["params"]["clientInfo"]["name"]
                            .as_str()
                            .unwrap_or("MCP")
                            .chars()
                            .take(256)
                            .collect();
                        updates.write().unwrap_or_else(|p| p.into_inner()).agent =
                            Some(agent.clone());
                    }
                    // Every request obtains a fresh token from the app's session.
                    let response = async {
                        let mut send = client
                            .http
                            .post(format!("{}/mcp", client.base_url))
                            .header("Accept", "application/json, text/event-stream")
                            .header("MCP-Protocol-Version", "2025-11-25")
                            .header("Kabacus-Agent-Name", &agent)
                            .json(&request);
                        if let Some(auth) = &client.auth {
                            send = send.bearer_auth(auth.token().await?);
                        }
                        let result = send.send().await.map_err(error)?;
                        if result.status() == reqwest::StatusCode::ACCEPTED {
                            return Ok(None);
                        }
                        let result = result
                            .error_for_status()
                            .map_err(error)?
                            .json::<serde_json::Value>()
                            .await
                            .map_err(error)?;
                        Ok::<_, ClientError>(Some(result))
                    }
                    .await;
                    let response = match response {
                        Ok(Some(response)) => response,
                        Ok(None) => continue,
                        Err(error) => {
                            serde_json::json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32603,"message":error.to_string()}})
                        }
                    };
                    if request["method"] == "tools/call" && response["result"]["isError"] != true {
                        let data = &response["result"]["structuredContent"];
                        if data["dry_run"] == false {
                            fn ids(value: &serde_json::Value, found: &mut Vec<u64>) {
                                if let Some(id) = value.get("id").and_then(|v| v.as_u64()) {
                                    found.push(id);
                                }
                                if let Some(array) = value.as_array() {
                                    for child in array {
                                        ids(child, found);
                                    }
                                } else if let Some(object) = value.as_object() {
                                    for key in ["transactions", "after", "before", "ops", "written"]
                                    {
                                        if let Some(child) = object.get(key) {
                                            ids(child, found);
                                        }
                                    }
                                }
                            }
                            let mut changed = Vec::new();
                            ids(data, &mut changed);
                            changed.sort_unstable();
                            changed.dedup();
                            let mut activity = updates.write().unwrap_or_else(|p| p.into_inner());
                            activity.changed = changed;
                            activity.changed_by = Some(agent.clone());
                            activity.expires_at = clock() + 8000;
                        }
                    }
                    if request.get("id").is_some()
                        && write
                            .write_all(format!("{response}\n").as_bytes())
                            .await
                            .is_err()
                    {
                        break;
                    }
                }
                updates.write().unwrap_or_else(|p| p.into_inner()).agent = None;
            }
        });
        Ok(Self {
            port,
            activity,
            task,
        })
    }
    pub async fn start(books: LocalBooks, port: u16) -> Result<Self, ClientError> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(error)?;
        let port = listener.local_addr().map_err(error)?.port();
        let activity = Arc::new(RwLock::new(AgentActivity::default()));
        let updates = activity.clone();
        let task = tokio::spawn(async move {
            // Each accepted connection finishes before another can control this window.
            while let Ok((stream, _)) = listener.accept().await {
                let mut who = books.caller.clone();
                who.actor = "mcp".into();
                who.allow_rewrites = false;
                let handler = kabacus_mcp::KabacusMcp::new(
                    books.service.clone(),
                    Some(who),
                    None,
                    Arc::new(kabacus_mcp::ToolLists::new()),
                );
                let (read, write) = stream.into_split();
                let Ok(server) = handler.serve((read, write)).await else {
                    continue;
                };
                let agent = server
                    .peer()
                    .peer_info()
                    .map(|i| i.client_info.name.clone())
                    .unwrap_or_else(|| "MCP".into());
                updates.write().unwrap_or_else(|p| p.into_inner()).agent = Some(agent.clone());
                let mut head = books
                    .service
                    .store
                    .revision(books.service.ledger)
                    .await
                    .unwrap_or(0);
                let waiting = server.waiting();
                tokio::pin!(waiting);
                loop {
                    tokio::select! {
                        _=&mut waiting=>break,
                        _=tokio::time::sleep(Duration::from_millis(100))=>{
                            let Ok(current)=books.service.store.revision(books.service.ledger).await else {continue};
                            if current==head {continue;}
                            let mut changed=Vec::new();
                            for revision in head+1..=current {
                                if let Ok(record)=books.service.store.change(books.service.ledger,revision).await {
                                    for change in record.changes {
                                        if let Some(Row::Transaction(tx))=change.after.or(change.before) { changed.push(tx.id); }
                                    }
                                }
                            }
                            changed.sort_unstable();changed.dedup();head=current;
                            let mut activity=updates.write().unwrap_or_else(|p|p.into_inner());
                            activity.changed=changed;activity.changed_by=Some(agent.clone());activity.expires_at=clock()+8000;
                        }
                    }
                }
                updates.write().unwrap_or_else(|p| p.into_inner()).agent = None;
            }
        });
        Ok(Self {
            port,
            activity,
            task,
        })
    }
}
fn clock() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn error(error: impl std::fmt::Display) -> ClientError {
    ClientError::Request(error.to_string())
}

pub async fn proxy_stdio(port: u16) -> Result<(), ClientError> {
    let stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(error)?;
    let (mut read, mut write) = stream.into_split();
    let input = async {
        tokio::io::copy(&mut tokio::io::stdin(), &mut write).await?;
        write.shutdown().await
    };
    let output = async {
        tokio::io::copy(&mut read, &mut tokio::io::stdout())
            .await
            .map(|_| ())
    };
    tokio::try_join!(input, output).map_err(error)?;
    Ok(())
}
