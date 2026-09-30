// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Through the real server: a counterparty statement exported as
//! CSV and PDF, named `statement-<name>-<end-date>.<ext>`.

use chrono::NaiveDate;
use kabacus_client::types::{AddTransactionBody, DraftAmount, DraftPostingBody};
use kabacus_client::{ClientError, KabacusClient};
use kabacus_core::StatementHeader;
use kabacus_testkit::{spawn_server, TestServer};

fn posting(account: &str, cents: Option<i128>) -> DraftPostingBody {
    DraftPostingBody {
        account: account.to_string(),
        amount: cents.map(|mantissa| DraftAmount {
            commodity: "USD".to_string(),
            mantissa: mantissa.into(),
            places: 2,
        }),
        status: String::new(),
        balance_assertion: None,
        clear_balance_assertion: false,
    }
}

async fn add(
    client: &KabacusClient,
    date: &str,
    description: &str,
    postings: Vec<DraftPostingBody>,
) {
    let head = client.version().await.unwrap().head.unwrap();
    client
        .add_transaction(
            &head,
            AddTransactionBody {
                date: date.to_string(),
                description: description.to_string(),
                postings,
                tags: vec![],
                dry_run: false,
            },
        )
        .await
        .unwrap();
}

/// Jordan owes half of a dinner, then pays part of it back.
async fn books_with_a_statement() -> (TestServer, KabacusClient) {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    add(
        &client,
        "2026-02-03",
        "Dinner, split",
        vec![
            posting("expenses:groceries", Some(4200)),
            posting("assets:receivable:jordan", Some(4200)),
            posting("assets:checking", None),
        ],
    )
    .await;
    add(
        &client,
        "2026-02-20",
        "Jordan paid me back",
        vec![
            posting("assets:checking", Some(2500)),
            posting("assets:receivable:jordan", None),
        ],
    )
    .await;
    (server, client)
}
#[tokio::test]
async fn exports_a_statement_as_csv() {
    let (_server, client) = books_with_a_statement().await;
    let file = client
        .export_statement("jordan", "2026-02-01", "2026-02-28", false, "csv", None)
        .await
        .unwrap();
    assert_eq!(file.filename, "statement-jordan-2026-02-28.csv");
    assert!(file.content_type.starts_with("text/csv"));
    let csv = String::from_utf8(file.bytes).unwrap();
    let rows: Vec<&str> = csv.lines().collect();
    assert_eq!(rows[0], "Date,Description,Charges,Payments,Balance");
    assert_eq!(rows[1], "2026-02-01,Opening balance,,,0.00");
    assert_eq!(rows[2], "2026-02-03,\"Dinner, split\",42.00,,42.00");
    assert_eq!(rows[3], "2026-02-20,Jordan paid me back,,25.00,17.00");
    assert_eq!(rows[4], "2026-02-28,Balance due from jordan,,,17.00");

    // The same closing balance as the statement itself.
    let statement = client
        .statement("jordan", "2026-02-01", "2026-02-28", false)
        .await
        .unwrap();
    assert_eq!(statement.closing_balance.exact, "17.00");
}
#[tokio::test]
async fn exports_a_statement_as_pdf_that_a_real_reader_can_read() {
    let (server, client) = books_with_a_statement().await;
    let header = StatementHeader {
        sender_name: "Sam Sender".to_string(),
        sender_address: vec!["1 Example Way".to_string(), "Springfield".to_string()],
        statement_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
    };
    let file = client
        .export_statement(
            "jordan",
            "2026-02-01",
            "2026-02-28",
            false,
            "pdf",
            Some(&header),
        )
        .await
        .unwrap();
    assert_eq!(file.filename, "statement-jordan-2026-02-28.pdf");
    assert_eq!(file.content_type, "application/pdf");

    let pdf_path = server.dir.path().join(&file.filename);
    std::fs::write(&pdf_path, &file.bytes).unwrap();
    let output = std::process::Command::new("pdftotext")
        .arg("-layout")
        .arg(&pdf_path)
        .arg("-")
        .output()
        .expect("pdftotext (poppler-utils) is needed for this test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Sam Sender",
        "Springfield",
        "Statement for jordan",
        "Statement date: 2026-03-01",
        "Dinner, split",
        "$42.00",
        "$25.00",
        "Balance due from jordan",
        "$17.00",
        "Page 1 of 1",
    ] {
        assert!(
            text.contains(expected),
            "{expected:?} missing from:\n{text}"
        );
    }
}
#[tokio::test]
async fn an_unknown_format_is_refused() {
    let (_server, client) = books_with_a_statement().await;
    let result = client
        .export_statement("jordan", "2026-02-01", "2026-02-28", false, "docx", None)
        .await;
    assert!(
        matches!(result, Err(ClientError::Invalid { .. })),
        "{result:?}"
    );
}
