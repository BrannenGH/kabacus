// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Command scripts become one validated, atomic batch.

use crate::{
    batch::{BatchBody, BatchOp},
    caller::Caller,
    error::Error,
    service, Service,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Step {
    pub command: String,
    #[serde(rename = "as")]
    pub name: Option<String>,
    #[serde(default)]
    pub params: Value,
}

pub async fn run(
    state: &Service,
    caller: &dyn Caller,
    steps: Vec<Step>,
    head: String,
    save: bool,
) -> Result<Value, Error> {
    let accounts = service::accounts(state, caller, None).await?;
    let commodity = service::books_commodity(state).await?;
    let resolve = |name: &str| -> String {
        accounts
            .iter()
            .find(|a| {
                a.account.eq_ignore_ascii_case(name) || a.display_name.eq_ignore_ascii_case(name)
            })
            .map(|a| a.account.clone())
            .unwrap_or_else(|| name.to_string())
    };
    let mut references = HashMap::new();
    let mut ops = Vec::new();
    for step in steps {
        let mut args = step.params;
        if !args.is_object() {
            return Err(service::invalid(
                "params",
                "Command parameters must be an object".into(),
            ));
        }
        let tool = match step.command.as_str() {
            "transaction.spent" | "transaction.received" | "transaction.transfer" => {
                let from = args["from"].as_str().ok_or_else(|| service::invalid("from", "Choose the source account".into()))?;
                let to = args["for"].as_str().or_else(|| args["to"].as_str()).ok_or_else(|| service::invalid("to", "Choose the destination account".into()))?;
                let from = resolve(from); let to = resolve(to);
                let amount = service::AmountInput(args["amount"].clone()).to_money(&commodity, "amount")?;
                if !amount.is_positive() { return Err(service::invalid("amount", "Enter a positive amount".into())); }
                let description = args["description"].as_str().or_else(|| args["payee"].as_str()).unwrap_or("Transfer").to_string();
                if let Some(name) = &step.name { references.insert(name.clone(), from.clone()); }
                args = json!({"date": args["date"], "description": description, "postings": [
                    {"account": to, "amount": amount.to_string()}, {"account": from, "amount": (-amount).to_string()}
                ], "tags": args.get("tags").cloned().unwrap_or(json!([]))});
                "add_transaction"
            },
            "transaction.setCleared" => {
                if args.get("account").is_none() {
                    let name = args["id"]["$ref"].as_str().unwrap_or_default();
                    if let Some(account) = references.get(name) { args["account"] = json!(account); }
                }
                if args.get("status").is_none() { args["status"] = json!("cleared"); }
                "set_cleared"
            },
            "transaction.edit" => "edit_transaction",
            "transaction.void" => "void_transaction",
            "transaction.postings" => "add_transaction",
            "reserve.setAside" | "reserve.move" | "reserve.release" => "allocate",
            "card.purchase" => "record_card_purchase",
            "card.pay" => "pay_card",
            other => crate::catalog::command_tool(other),
        }.to_string();
        for field in [
            "from",
            "to",
            "account",
            "card",
            "expense_account",
            "source",
            "paid_with",
            "category",
        ] {
            if let Some(name) = args[field].as_str() {
                args[field] = json!(resolve(name));
            }
        }
        if step.command == "reconcile.finish" && args.get("mode").is_none() {
            args["mode"] = json!("assert");
        }
        ops.push(BatchOp {
            tool,
            args,
            client_ref: step.name,
        });
    }
    crate::batch::batch(
        state,
        caller,
        BatchBody {
            head,
            dry_run: Some(!save),
            commit: Some("single".into()),
            mode: Some("atomic".into()),
            batch_id: None,
            ops,
        },
    )
    .await
}
