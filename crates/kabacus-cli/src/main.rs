// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::{CommandFactory, Parser, Subcommand};
use kabacus_client::{books::computer_user, Books, BooksRequest, LocalBooks};
use kabacus_service::local::LocalCaller;
use kabacus_store::BookStore;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};

#[derive(Parser)]
#[command(
    name = "kabacus-cli",
    version,
    about = "Inspect and change local Kabacus books"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    allow_rewrites: bool,
}
#[derive(Subcommand)]
enum Command {
    Info {
        books: PathBuf,
    },
    New {
        books: PathBuf,
        #[arg(long, default_value = "USD")]
        currency: String,
        #[arg(long, default_value = "assets:bank:checking")]
        account: String,
        #[arg(long, default_value = "Books")]
        name: String,
    },
    Report {
        books: PathBuf,
        name: String,
        #[arg(long)]
        period: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        csv: bool,
    },
    Export {
        books: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    History {
        books: PathBuf,
        #[arg(long)]
        diff: Option<String>,
    },
    Run {
        script: PathBuf,
        #[arg(long)]
        books: PathBuf,
        #[arg(long)]
        save: bool,
        #[arg(long)]
        head: Option<String>,
    },
    Commands {
        #[arg(long)]
        filter: Option<String>,
    },
    /// Preview or save a bank or card statement import.
    ImportStatement {
        books: PathBuf,
        statement: PathBuf,
        #[arg(long)]
        account: String,
        #[arg(long)]
        head: String,
        /// JSON file with CSV mapping, statement selection, or review decisions.
        #[arg(long)]
        options: Option<PathBuf>,
        #[arg(long)]
        save: bool,
    },
    /// Manage a SimpleFIN connection and sync linked accounts.
    Simplefin {
        books: PathBuf,
        #[command(subcommand)]
        action: SimplefinCommand,
    },
    Mcp {
        #[arg(long, required_unless_present = "attach", conflicts_with = "attach")]
        books: Vec<PathBuf>,
        #[arg(long, conflicts_with = "books")]
        attach: Option<u16>,
        #[arg(long)]
        read_only: bool,
    },
    Completions {
        #[arg(value_parser = ["bash", "zsh", "fish"])]
        shell: String,
    },
    Man,
}
#[derive(Subcommand)]
enum SimplefinCommand {
    Status,
    /// Read a one-time SimpleFIN setup token from standard input.
    Connect {
        #[arg(long)]
        replace: bool,
        #[arg(long)]
        preview: bool,
    },
    Disconnect,
    Accounts {
        #[arg(long)]
        refresh: bool,
    },
    Link {
        account: String,
        #[arg(long)]
        head: String,
        #[arg(long, requires = "remote_id", conflicts_with = "unlink")]
        conn_id: Option<String>,
        #[arg(long, requires = "conn_id", conflicts_with = "unlink")]
        remote_id: Option<String>,
        #[arg(long)]
        unlink: bool,
    },
    Sync {
        #[arg(long)]
        head: String,
        /// Review and confirm the import in this process.
        #[arg(long)]
        save: bool,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        refresh: bool,
        /// JSON options with per-account date starts and review decisions.
        #[arg(long)]
        options: Option<PathBuf>,
    },
}

async fn bank_operation(books: &LocalBooks, action: SimplefinCommand) -> Result<Value> {
    let (method, path, head, body) = match action {
        SimplefinCommand::Status => ("GET", "/simplefin/connection".into(), None, Value::Null),
        SimplefinCommand::Connect { replace, preview } => {
            let mut token = String::new();
            std::io::stdin().read_line(&mut token)?;
            if !preview {
                eprintln!("This file will hold a bank credential. Anyone with the file can read the connected transactions until you revoke access at your SimpleFIN server.");
            }
            (
                "PUT",
                "/simplefin/connection".into(),
                None,
                json!({"token":token.trim(), "replace":replace, "dry_run":preview}),
            )
        }
        SimplefinCommand::Disconnect => {
            ("DELETE", "/simplefin/connection".into(), None, Value::Null)
        }
        SimplefinCommand::Accounts { refresh } => (
            "GET",
            format!("/simplefin/accounts?refresh={refresh}"),
            None,
            Value::Null,
        ),
        SimplefinCommand::Link {
            account,
            head,
            conn_id,
            remote_id,
            unlink,
        } => {
            let remote = match conn_id.zip(remote_id) {
                Some((conn_id, id)) => json!({"conn_id":conn_id, "id":id}),
                None if unlink => Value::Null,
                None => bail!("Give both --conn-id and --remote-id, or use --unlink"),
            };
            (
                "PUT",
                format!(
                    "/accounts/{}/simplefin-link",
                    kabacus_client::urlencode(&account)
                ),
                Some(head),
                json!({"remote":remote}),
            )
        }
        SimplefinCommand::Sync {
            head,
            save,
            since,
            refresh,
            options,
        } => {
            let mut body = match options {
                Some(path) => serde_json::from_reader(std::fs::File::open(path)?)?,
                None => json!({}),
            };
            let fields = body
                .as_object_mut()
                .context("Sync options must be a JSON object")?;
            fields.insert("dry_run".into(), json!(true));
            fields.remove("snapshot");
            if let Some(since) = since {
                fields.insert("since".into(), json!(since));
            }
            if refresh {
                fields.insert("refresh".into(), json!(true));
            }
            let mut request = BooksRequest {
                method: "POST".into(),
                path: "/simplefin/sync".into(),
                head: Some(head),
                body,
            };
            let reviewed = bank_request(books, request.clone()).await?;
            if !save
                || reviewed["counts"]["new"].as_u64().unwrap_or_default()
                    + reviewed["counts"]["matched"].as_u64().unwrap_or_default()
                    == 0
            {
                return Ok(reviewed);
            }
            eprintln!("{}", serde_json::to_string_pretty(&reviewed)?);
            eprintln!("Import the reviewed transactions? [y/N]");
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            if !answer.trim().eq_ignore_ascii_case("y")
                && !answer.trim().eq_ignore_ascii_case("yes")
            {
                return Ok(reviewed);
            }
            request.body["snapshot"] = reviewed["snapshot"].clone();
            request.body["dry_run"] = json!(false);
            return bank_request(books, request).await;
        }
    };
    bank_request(
        books,
        BooksRequest {
            method: method.into(),
            path,
            head,
            body,
        },
    )
    .await
}

async fn bank_request(books: &LocalBooks, request: BooksRequest) -> Result<Value> {
    let response = books.request(request).await?;
    let body: Value = serde_json::from_slice(&response.body)?;
    if response.status >= 400 {
        bail!("{}", body["message"].as_str().unwrap_or("Request failed"));
    }
    Ok(body)
}

fn caller(allow: bool, actor: &str) -> LocalCaller {
    let mut caller = LocalCaller::new(computer_user(), actor);
    caller.allow_rewrites = allow;
    caller
}
async fn call(books: &LocalBooks, path: String) -> Result<Value> {
    let response = books
        .request(BooksRequest {
            method: "GET".into(),
            path,
            head: None,
            body: Value::Null,
        })
        .await?;
    let body: Value = serde_json::from_slice(&response.body)?;
    if response.status >= 400 {
        bail!("{}", body["message"].as_str().unwrap_or("Request failed"));
    }
    Ok(body)
}
fn print(value: &Value, json: bool) {
    if json {
        println!("{value}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        );
    }
}
fn csv(value: &Value) {
    fn flatten(prefix: &str, value: &Value, row: &mut std::collections::BTreeMap<String, String>) {
        if let Value::Object(values) = value {
            for (key, value) in values {
                flatten(
                    &if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    },
                    value,
                    row,
                );
            }
        } else {
            row.insert(
                prefix.into(),
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string()),
            );
        }
    }
    let rows: Vec<_> = value
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![value.clone()])
        .iter()
        .map(|v| {
            let mut row = std::collections::BTreeMap::new();
            flatten("", v, &mut row);
            row
        })
        .collect();
    let columns: std::collections::BTreeSet<_> =
        rows.iter().flat_map(|r| r.keys().cloned()).collect();
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    println!(
        "{}",
        columns
            .iter()
            .map(|s| quote(s))
            .collect::<Vec<_>>()
            .join(",")
    );
    for row in rows {
        println!(
            "{}",
            columns
                .iter()
                .map(|c| quote(row.get(c).map(String::as_str).unwrap_or_default()))
                .collect::<Vec<_>>()
                .join(",")
        );
    }
}
async fn run(args: Args) -> Result<()> {
    let local = caller(args.allow_rewrites, "cli");
    match args.command {
        Command::New {
            books,
            currency,
            account,
            name,
        } => {
            let books = LocalBooks::create(books, &name, &currency, &account, local).await?;
            print(&call(&books, "/version".into()).await?, args.json);
        }
        Command::Info { books } => {
            let books = LocalBooks::open(books, local).await?;
            print(
                &json!({"version": call(&books,"/version".into()).await?, "accounts": call(&books,"/accounts".into()).await?,
                "cash_on_hand": call(&books,"/cash-on-hand".into()).await?, "history": call(&books,"/history".into()).await?}),
                args.json,
            );
        }
        Command::History { books, diff } => {
            let books = LocalBooks::open(books, local).await?;
            if let Some(rev) = diff {
                let response = books
                    .request(BooksRequest {
                        method: "GET".into(),
                        path: format!("/history/{rev}"),
                        head: None,
                        body: Value::Null,
                    })
                    .await?;
                if response.status >= 400 {
                    bail!("{}", String::from_utf8_lossy(&response.body));
                }
                let diff = String::from_utf8_lossy(&response.body);
                if args.json {
                    print(&json!({"revision":rev,"diff":diff}), true);
                } else {
                    println!("{diff}");
                }
            } else {
                print(&call(&books, "/history".into()).await?, args.json);
            }
        }
        Command::Report {
            books,
            name,
            period,
            account,
            csv: as_csv,
        } => {
            let books = LocalBooks::open(books, local).await?;
            let mut path = match name.as_str() {
                "cash-on-hand" => "/cash-on-hand".to_string(),
                "register" => format!(
                    "/register/{}",
                    kabacus_client::urlencode(
                        account.as_deref().context("register requires --account")?
                    )
                ),
                "balance-sheet"
                | "income-statement"
                | "cash-flow"
                | "categories"
                | "cash-on-hand-history"
                | "receivables" => format!("/reports/{name}"),
                _ => bail!("Unknown report: {name}"),
            };
            if let Some(period) = period {
                let (from, to) = period
                    .split_once("..")
                    .context("--period uses FROM..TO dates")?;
                path.push_str(&format!("?from={from}&to={to}"));
            }
            let value = call(&books, path).await?;
            if as_csv {
                csv(&value);
            } else {
                print(&value, args.json);
            }
        }
        Command::Export { books, output } => {
            let books = LocalBooks::open(books, local).await?;
            let loaded = books.service.store.load(books.service.ledger, None).await?;
            let text = kabacus_ledger::render_book(&loaded.book);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            std::io::Write::write_all(&mut file, text.as_bytes())?;
            print(&json!({"output":output}), args.json);
        }
        Command::Run {
            script,
            books,
            save,
            head,
        } => {
            let steps = serde_json::from_slice(&std::fs::read(script)?)?;
            let books = LocalBooks::open(books, local).await?;
            let head = head.unwrap_or(
                books
                    .service
                    .store
                    .revision(books.service.ledger)
                    .await?
                    .to_string(),
            );
            let result =
                kabacus_service::script::run(&books.service, &books.caller, steps, head, save)
                    .await?;
            for step in result["ops"].as_array().into_iter().flatten() {
                println!(
                    "{}",
                    json!({"head":result["head"],"dry_run":result["dry_run"],"result":step})
                );
            }
        }
        Command::Commands { filter } => {
            let mut catalog = kabacus_service::catalog::commands();
            if let Some(filter) = filter {
                for key in ["add", "commands"] {
                    if let Some(items) = catalog[key].as_array_mut() {
                        items.retain(|c| {
                            c.to_string()
                                .to_lowercase()
                                .contains(&filter.to_lowercase())
                        });
                    }
                }
            }
            print(&catalog, args.json);
        }
        Command::ImportStatement {
            books,
            statement,
            account,
            head,
            options,
            save,
        } => {
            use std::io::Read;
            let format = statement
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .context("Choose a file with an OFX, QFX, or CSV extension")?
                .to_ascii_lowercase();
            let mut body = match options {
                Some(path) => serde_json::from_reader(std::fs::File::open(path)?)?,
                None => json!({}),
            };
            let fields = body
                .as_object_mut()
                .context("Import options must be a JSON object")?;
            let max_bytes = kabacus_service::bank_import::Limits::default().max_bytes;
            let mut bytes = Vec::new();
            std::fs::File::open(&statement)?
                .take(max_bytes as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > max_bytes {
                bail!("The statement exceeds the import limit");
            }
            fields.insert("format".into(), json!(format));
            fields.insert("content_base64".into(), json!(STANDARD.encode(bytes)));
            fields.insert(
                "filename".into(),
                json!(statement.file_name().and_then(std::ffi::OsStr::to_str)),
            );
            fields.insert("dry_run".into(), json!(!save));
            let body = serde_json::from_value(body)?;
            let local = LocalBooks::open(books, local).await?;
            let result = kabacus_service::bank_import::import_statement(
                &local.service,
                &local.caller,
                &account,
                &head,
                body,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.body().message))?;
            print(&result, args.json);
        }
        Command::Simplefin { books, action } => {
            let books = LocalBooks::open(books, local).await?;
            print(&bank_operation(&books, action).await?, args.json);
        }
        Command::Mcp {
            books,
            attach,
            read_only,
        } => {
            if let Some(port) = attach {
                return kabacus_client::control::proxy_stdio(port)
                    .await
                    .map_err(Into::into);
            }
            let mut opened = Vec::new();
            for path in books {
                let mut who = caller(args.allow_rewrites, "mcp");
                who.read_only = read_only;
                opened.push(LocalBooks::open(path, who).await?);
            }
            let books = opened.first().context("Provide --books or --attach")?;
            let handler = kabacus_mcp::KabacusMcp::new(
                books.service.clone(),
                Some(books.caller.clone()),
                None,
                Arc::new(kabacus_mcp::ToolLists::new()),
            )
            .with_books(opened.iter().map(|b| {
                (
                    b.path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    b.service.clone(),
                    b.caller.clone(),
                )
            }));
            let server = handler.serve(rmcp::transport::stdio()).await?;
            server.waiting().await?;
        }
        Command::Completions { shell } => {
            let shell: clap_complete::Shell = shell.parse().map_err(anyhow::Error::msg)?;
            clap_complete::generate(
                shell,
                &mut Args::command(),
                "kabacus-cli",
                &mut std::io::stdout(),
            );
        }
        Command::Man => {
            clap_mangen::Man::new(Args::command()).render(&mut std::io::stdout())?;
        }
    }
    Ok(())
}
#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Err(error) = run(args).await {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
