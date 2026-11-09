// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

/// One route of the API and the MCP tools that do the same (the
/// MCP endpoint has every function the API has).
#[derive(Debug, serde::Serialize)]
pub struct Endpoint {
    pub method: &'static str,
    pub path: &'static str,
    pub mcp_tools: &'static [&'static str],
}

/// Ledger API routes, besides MCP and UI assets. Each route added to
/// [`router`] is added here too, with the tools that mirror it.
pub const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        method: "GET",
        path: "/commands",
        mcp_tools: &["get_commands"],
    },
    Endpoint {
        method: "POST",
        path: "/splits",
        mcp_tools: &["record_shared_expense"],
    },
    Endpoint {
        method: "GET",
        path: "/version",
        mcp_tools: &["get_version"],
    },
    Endpoint {
        method: "GET",
        path: "/commodity",
        mcp_tools: &["get_commodity"],
    },
    Endpoint {
        method: "GET",
        path: "/transactions",
        mcp_tools: &["search_transactions"],
    },
    Endpoint {
        method: "POST",
        path: "/transactions",
        mcp_tools: &["add_transaction"],
    },
    Endpoint {
        method: "GET",
        path: "/transactions/{id}",
        mcp_tools: &["get_transaction"],
    },
    Endpoint {
        method: "PATCH",
        path: "/transactions/{id}",
        mcp_tools: &["edit_transaction"],
    },
    Endpoint {
        method: "POST",
        path: "/batch",
        mcp_tools: &["batch"],
    },
    Endpoint {
        method: "PUT",
        path: "/transactions/{id}",
        mcp_tools: &["rewrite_transaction"],
    },
    Endpoint {
        method: "DELETE",
        path: "/transactions/{id}",
        mcp_tools: &["delete_transaction"],
    },
    Endpoint {
        method: "POST",
        path: "/transactions/{id}/corrections",
        mcp_tools: &["correct_transaction", "void_transaction"],
    },
    Endpoint {
        method: "PATCH",
        path: "/transactions/{id}/status",
        mcp_tools: &["set_cleared"],
    },
    Endpoint {
        method: "PUT",
        path: "/assertions",
        mcp_tools: &["set_balance_assertion"],
    },
    Endpoint {
        method: "POST",
        path: "/assertions/inclusive",
        mcp_tools: &["convert_money_assertions"],
    },
    Endpoint {
        method: "GET",
        path: "/history",
        mcp_tools: &["get_history"],
    },
    Endpoint {
        method: "GET",
        path: "/history/{rev}",
        mcp_tools: &["get_history_diff"],
    },
    Endpoint {
        method: "GET",
        path: "/history/{rev}/transactions",
        mcp_tools: &["get_transactions_at"],
    },
    Endpoint {
        method: "GET",
        path: "/cash-on-hand",
        mcp_tools: &["cash_on_hand"],
    },
    Endpoint {
        method: "POST",
        path: "/allocations",
        mcp_tools: &["allocate"],
    },
    Endpoint {
        method: "GET",
        path: "/cards/reserves",
        mcp_tools: &["card_reserves"],
    },
    Endpoint {
        method: "POST",
        path: "/cards/purchases",
        mcp_tools: &["record_card_purchase"],
    },
    Endpoint {
        method: "POST",
        path: "/cards/payments",
        mcp_tools: &["pay_card"],
    },
    Endpoint {
        method: "GET",
        path: "/counterparties",
        mcp_tools: &["list_counterparties"],
    },
    Endpoint {
        method: "GET",
        path: "/counterparties/{name}/statement",
        mcp_tools: &["counterparty_statement", "export_statement"],
    },
    Endpoint {
        method: "GET",
        path: "/accounts",
        mcp_tools: &["list_accounts"],
    },
    Endpoint {
        method: "POST",
        path: "/accounts",
        mcp_tools: &["declare_account"],
    },
    Endpoint {
        method: "POST",
        path: "/accounts/rename",
        mcp_tools: &["rename_account"],
    },
    Endpoint {
        method: "PUT",
        path: "/accounts/{account}/type",
        mcp_tools: &["set_account_type"],
    },
    Endpoint {
        method: "DELETE",
        path: "/accounts/{account}",
        mcp_tools: &["remove_account"],
    },
    Endpoint {
        method: "GET",
        path: "/accounts/kinds",
        mcp_tools: &["account_kinds"],
    },
    Endpoint {
        method: "POST",
        path: "/reconcile",
        mcp_tools: &["reconcile"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/income-statement",
        mcp_tools: &["income_statement"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/balance-sheet",
        mcp_tools: &["balance_sheet"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/categories",
        mcp_tools: &["category_breakdown"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/cash-on-hand-history",
        mcp_tools: &["cash_on_hand_history"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/cash-flow",
        mcp_tools: &["cash_flow"],
    },
    Endpoint {
        method: "GET",
        path: "/reports/receivables",
        mcp_tools: &["receivables_activity"],
    },
    Endpoint {
        method: "GET",
        path: "/payees",
        mcp_tools: &["list_payees"],
    },
    Endpoint {
        method: "GET",
        path: "/register/{account}",
        mcp_tools: &["get_register"],
    },
    // `.kabacus` files are binary: API only, no MCP tools.
    Endpoint {
        method: "GET",
        path: "/ledgers/{id}/export",
        mcp_tools: &[],
    },
    Endpoint {
        method: "POST",
        path: "/ledgers/import",
        mcp_tools: &[],
    },
];

#[derive(Debug, serde::Serialize)]
pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub tool: &'static str,
    pub key: &'static str,
}

pub const ADD_COMMANDS: &[Command] = &[
    Command {
        id: "transaction.spent",
        label: "Spent",
        method: "POST",
        path: "/transactions",
        tool: "add_transaction",
        key: "1",
    },
    Command {
        id: "transaction.received",
        label: "Received",
        method: "POST",
        path: "/transactions",
        tool: "add_transaction",
        key: "2",
    },
    Command {
        id: "card.purchase",
        label: "Card purchase",
        method: "POST",
        path: "/cards/purchases",
        tool: "record_card_purchase",
        key: "3",
    },
    Command {
        id: "transaction.transfer",
        label: "Transfer",
        method: "POST",
        path: "/transactions",
        tool: "add_transaction",
        key: "4",
    },
    Command {
        id: "reserve.setAside",
        label: "Set aside",
        method: "POST",
        path: "/allocations",
        tool: "allocate",
        key: "5",
    },
    Command {
        id: "card.pay",
        label: "Pay a card",
        method: "POST",
        path: "/cards/payments",
        tool: "pay_card",
        key: "6",
    },
    Command {
        id: "split.record",
        label: "Split with someone",
        method: "POST",
        path: "/splits",
        tool: "record_shared_expense",
        key: "7",
    },
    Command {
        id: "transaction.postings",
        label: "Postings",
        method: "POST",
        path: "/transactions",
        tool: "add_transaction",
        key: "8",
    },
];

pub fn commands() -> serde_json::Value {
    let mut actions = Vec::new();
    for endpoint in ENDPOINTS {
        for &tool in endpoint.mcp_tools {
            let id = match tool {
                "add_transaction" => "transaction.postings",
                "edit_transaction" => "transaction.edit",
                "set_cleared" => "transaction.setCleared",
                "void_transaction" => "transaction.void",
                "allocate" => "reserve.setAside",
                "pay_card" => "card.pay",
                "record_card_purchase" => "card.purchase",
                "record_shared_expense" => "split.record",
                "reconcile" => "reconcile.finish",
                other => other,
            };
            actions.push(serde_json::json!({"id":id,"label":tool.replace('_'," "),"menu":"Books","shortcut":"","method":endpoint.method,"path":endpoint.path,"tool":tool,"parameters":parameters(tool)}));
        }
    }
    for command in ADD_COMMANDS {
        actions.retain(|action| action["id"] != command.id);
        actions.push(serde_json::json!({"id":command.id,"label":command.label,"menu":"Add","shortcut":command.key,"method":command.method,"path":command.path,"tool":command.tool,"parameters":parameters(command.tool),"script_parameters":script_parameters(command.id,command.tool)}));
    }
    for (id, label, key) in [
        ("file.newWindow", "New Window", "Ctrl+Shift+N"),
        ("file.new", "New Books…", ""),
        ("file.open", "Open Books…", "Ctrl+O"),
        ("file.recent", "Open Recent", ""),
        ("file.saveCopy", "Save a Copy As…", "Ctrl+Shift+S"),
        ("file.close", "Close Window", "Ctrl+Shift+W"),
        ("file.connect", "Connect to a Server…", ""),
        ("app.quit", "Quit", "Ctrl+Q"),
    ] {
        actions.push(serde_json::json!({"id":id,"label":label,"menu":"File","shortcut":key,"method":null,"path":null,"tool":null,"parameters":{"type":"object"}}));
    }
    serde_json::json!({"add": ADD_COMMANDS, "commands":actions,"endpoints": ENDPOINTS})
}

fn parameters(tool: &str) -> serde_json::Value {
    use crate::commands::*;
    macro_rules! schema {
        ($ty:ty) => {
            serde_json::to_value(schemars::schema_for!($ty)).unwrap_or_default()
        };
    }
    match tool {
        "add_transaction" => schema!(AddTransactionParams),
        "edit_transaction" => schema!(EditTransactionParams),
        "set_cleared" => schema!(SetClearedParams),
        "correct_transaction" => schema!(CorrectTransactionParams),
        "void_transaction" => schema!(VoidTransactionParams),
        "allocate" => schema!(AllocateParams),
        "record_card_purchase" => schema!(RecordCardPurchaseParams),
        "pay_card" => schema!(PayCardParams),
        "record_shared_expense" => schema!(crate::split::SplitBody),
        "reconcile" => schema!(crate::reports::ReconcileParams),
        "batch" => schema!(crate::batch::BatchBody),
        "search_transactions" => schema!(crate::search::SearchQuery),
        "declare_account" => schema!(DeclareAccountParams),
        "rename_account" => schema!(RenameAccountParams),
        "remove_account" => schema!(RemoveAccountParams),
        "rewrite_transaction" => schema!(RewriteTransactionParams),
        "delete_transaction" => schema!(DeleteTransactionParams),
        "set_account_type" => schema!(SetAccountTypeParams),
        "set_balance_assertion" => schema!(SetBalanceAssertionParams),
        "convert_money_assertions" => schema!(ConvertMoneyAssertionsParams),
        "get_transaction" => schema!(GetTransactionParams),
        "get_register" => schema!(GetRegisterParams),
        "list_accounts" => schema!(ListAccountsParams),
        "list_payees" => schema!(ListPayeesParams),
        "get_balance" => schema!(GetBalanceParams),
        "counterparty_statement" => schema!(CounterpartyStatementParams),
        "export_statement" => schema!(ExportStatementParams),
        "cash_on_hand" => schema!(CashOnHandParams),
        "card_reserves" => schema!(CardReservesParams),
        "get_history_diff" => schema!(RevisionParams),
        "get_transactions_at" => schema!(TransactionsAtParams),
        "get_skill_guide" => schema!(GetSkillGuideParams),
        "income_statement" => schema!(crate::reports::IncomeStatementParams),
        "balance_sheet" => schema!(crate::reports::BalanceSheetParams),
        "category_breakdown" => schema!(crate::reports::CategoryBreakdownParams),
        "cash_flow" => schema!(crate::reports::CashFlowParams),
        "cash_on_hand_history" => schema!(crate::reports::CashHistoryParams),
        "receivables_activity" => schema!(crate::reports::ReceivablesParams),
        _ => schema!(NoParams),
    }
}

/// Stable ids used outside the transport-specific tool names.
pub fn command_tool(id: &str) -> &str {
    match id {
        "transaction.spent"
        | "transaction.received"
        | "transaction.transfer"
        | "transaction.postings" => "add_transaction",
        "transaction.edit" => "edit_transaction",
        "transaction.setCleared" => "set_cleared",
        "transaction.void" => "void_transaction",
        "reserve.setAside" | "reserve.move" | "reserve.release" => "allocate",
        "card.purchase" => "record_card_purchase",
        "card.pay" => "pay_card",
        "split.record" => "record_shared_expense",
        "reconcile.finish" => "reconcile",
        other => other,
    }
}

fn script_parameters(id: &str, tool: &str) -> serde_json::Value {
    if matches!(
        id,
        "transaction.spent" | "transaction.received" | "transaction.transfer"
    ) {
        return serde_json::json!({"type":"object","properties":{"date":{"type":"string","format":"date"},"from":{"type":"string"},"to":{"type":"string"},"for":{"type":"string"},"amount":{"type":"string","description":"Exact decimal amount in the books currency"},"payee":{"type":"string"},"description":{"type":"string"}},"required":["date","from","amount"],"anyOf":[{"required":["to"]},{"required":["for"]}]});
    }
    let mut schema = parameters(tool);
    if let Some(object) = schema.as_object_mut() {
        if let Some(required) = object.get_mut("required").and_then(|v| v.as_array_mut()) {
            required.retain(|v| v != "head" && v != "dry_run");
        }
        if let Some(properties) = object.get_mut("properties").and_then(|v| v.as_object_mut()) {
            properties.remove("head");
            properties.remove("dry_run");
        }
    }
    schema
}
