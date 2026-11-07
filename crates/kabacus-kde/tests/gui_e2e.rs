// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! GUI e2e: the real app, driven through AT-SPI, against a real
//! kabacus-server on synthetic books. Needs the app built with `--features
//! gui` and a display, so it's ignored by default; `scripts/gui-e2e` runs it
//! in the openSUSE gui container, under Xvfb with a private D-Bus session.

use kabacus_client::types::BatchOpBody;
use kabacus_client::KabacusClient;
use kabacus_kde::model;
use kabacus_store::BookStore;
use kabacus_testkit::provider::{FakeProvider, ProviderOptions};
use kabacus_testkit::{spawn_oauth_server, spawn_server};
use std::path::PathBuf;
use std::process::{Child, Command};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run tests/gui/driver.py with `args`, failing the test if it fails.
async fn drive(args: &[&str]) {
    let driver = repo_root().join("crates/kabacus-kde/tests/gui/driver.py");
    let status = tokio::process::Command::new("python3")
        .arg(driver)
        .args(args)
        .status()
        .await
        .unwrap();
    assert!(status.success(), "AT-SPI driver failed: {args:?}");
}

/// The window size UI changes are tested at.
const DESKTOP: &str = "1280x800";

/// Start the app against `socket`, with its settings in `config` (never
/// the user's own), at the desktop size.
fn launch(socket: &std::path::Path, config: &std::path::Path) -> App {
    launch_with(socket, config, &[])
}

/// [`launch`] at `size` (`WxH`).
fn launch_sized(socket: &std::path::Path, config: &std::path::Path, size: &str) -> App {
    launch_full(socket, config, &[], size)
}

/// [`launch`], with extra environment variables.
fn launch_with(socket: &std::path::Path, config: &std::path::Path, env: &[(&str, &str)]) -> App {
    launch_full(socket, config, env, DESKTOP)
}

fn launch_full(
    socket: &std::path::Path,
    config: &std::path::Path,
    env: &[(&str, &str)],
    size: &str,
) -> App {
    // Only set when the tests are built with --features gui.
    let Some(app) = option_env!("CARGO_BIN_EXE_kabacus") else {
        panic!("build the tests with --features gui (scripts/gui-e2e)");
    };
    App(Command::new(app)
        .arg(format!("--size={size}"))
        .env("QT_LINUX_ACCESSIBILITY_ALWAYS_ON", "1")
        .env("KABACUS_SOCKET", socket)
        .env("XDG_CONFIG_HOME", config)
        .envs(env.iter().copied())
        .spawn()
        .unwrap())
}

/// A stand-in for the system browser: Qt opens URLs with `$BROWSER` (or
/// `xdg-open`), and this one loads the page and follows the provider's
/// redirect back to the app, as a user signing in would. Returns the
/// directory holding it, for `PATH`, and its path.
fn fake_browser(dir: &std::path::Path) -> (String, String) {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("xdg-open");
    std::fs::write(
        &script,
        "#!/bin/sh\nexec python3 -c 'import sys, urllib.request; \
         urllib.request.urlopen(sys.argv[1]).read()' \"$1\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap());
    (path, script.display().to_string())
}

/// A fresh settings directory for one test.
fn config_dir(name: &str) -> PathBuf {
    let dir = socket_path(&format!("{name}-config"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A socket where the app looks by default, `$XDG_RUNTIME_DIR/kabacus`.
fn socket_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap()).join("kabacus");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("gui-e2e-{name}-{}.sock", std::process::id()))
}

/// Kills the app when the test ends, pass or fail.
struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn account_type_can_be_changed_from_the_desktop_register() {
    let server = spawn_server(|cfg| {
        cfg.auth.no_auth_roles = vec![
            "ledger-read".into(),
            "ledger-write".into(),
            "ledger-rewrite".into(),
        ];
    })
    .await;
    let _app = launch(&server.socket_path, &config_dir("account-type"));
    let client = KabacusClient::connect_unix(&server.socket_path);
    let original = server
        .state
        .store
        .load(server.state.ledger, None)
        .await
        .unwrap()
        .book
        .transactions;
    for (label, synthetic) in [("Synthetic account", true), ("Real account", false)] {
        drive(&["account-type", "Everyday Checking", label]).await;
        let accounts = client.list_accounts(None).await.unwrap();
        assert_eq!(
            accounts
                .iter()
                .find(|a| a.account == "assets:checking")
                .unwrap()
                .synthetic,
            synthetic
        );
        let stored = server
            .state
            .store
            .load(server.state.ledger, None)
            .await
            .unwrap()
            .book;
        assert_eq!(stored.transactions, original);
    }
}

// Adding a transaction and reading it back in the accounts and register.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn adding_a_transaction_in_the_app_writes_it_to_the_books() {
    let socket = socket_path("add");
    let server = spawn_server(|c| c.socket_path = socket.clone()).await;
    let _app = launch(&server.socket_path, &config_dir("add"));

    drive(&[
        "add",
        "Hardware Store",
        "12.34",
        "assets:checking",
        "expenses:household",
    ])
    .await;

    let client = KabacusClient::connect_unix(&server.socket_path);
    let rows = model::register(&client, "expenses:household")
        .await
        .unwrap();
    assert!(
        !rows.is_empty(),
        "not in expenses:household; all transactions: {:#?}",
        model::register(&client, model::ALL_TRANSACTIONS)
            .await
            .unwrap()
    );
    assert_eq!(rows[0].payee, "Hardware Store");
    assert_eq!(rows[0].account, "assets:checking");
    assert!(rows[0].inflow.contains("12.34"), "{:?}", rows[0]);
}

// Clicking a transaction opens it for editing in place.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn clicking_a_transaction_edits_it_in_place() {
    let socket = socket_path("edit");
    let server = spawn_server(|c| c.socket_path = socket.clone()).await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let id = model::add_simple(
        &client,
        &model::SimpleDraft {
            date: "2026-02-03".into(),
            payee: "Corner Market".into(),
            memo: String::new(),
            from_account: "assets:checking".into(),
            to_account: "expenses:groceries".into(),
            amount: "45.67".into(),
        },
    )
    .await
    .unwrap();
    let _app = launch(&server.socket_path, &config_dir("edit"));

    drive(&[
        "edit",
        "Everyday Checking",
        "Corner Market",
        "2026-02-03",
        "2026-01-28",
        "Hardware Store",
        "9.99",
        "expenses:household",
    ])
    .await;

    let rows = model::register(&client, "assets:checking").await.unwrap();
    let row = rows
        .iter()
        .find(|r| r.id == id)
        .expect("same transaction, edited");
    assert_eq!(row.date, "2026-01-28");
    assert_eq!(row.payee, "Hardware Store");
    assert_eq!(row.account, "expenses:household");
    assert!(row.outflow.contains("9.99"), "{row:?}");
    assert_eq!(rows.len(), 2, "edited in place: {rows:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn local_books_are_created_and_two_windows_restore() {
    let script = repo_root().join("crates/kabacus-kde/tests/gui/local_windows.py");
    let status = tokio::process::Command::new("python3")
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_kabacus"))
        .status()
        .await
        .unwrap();
    assert!(status.success());
}

// About exposes the application version and corresponding source.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn about_shows_the_version_and_source() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("about{i}")).await;
        let version = KabacusClient::connect_unix(&server.socket_path)
            .version()
            .await
            .unwrap();
        assert_eq!(version.commit.len(), 40, "{size}");
        let _app = launch_sized(&server.socket_path, &config_dir(&format!("about{i}")), size);
        drive(&["about", &version.server_version, &version.source_url]).await;
    }
}

// Signing in through the browser, the refresh token kept in the Secret
// Service (never the settings file), and a restart resuming without the
// browser.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn signing_in_through_the_browser_is_remembered_across_restarts() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;
    let config = config_dir("signin");
    let (path, browser) = fake_browser(&config);
    let env = [("PATH", path.as_str()), ("BROWSER", browser.as_str())];

    let app = launch_with(&server.socket_path, &config, &env);
    drive(&["signin", "alex"]).await;
    drop(app);

    assert_eq!(provider.tokens_issued(), 1);
    let settings = std::fs::read_to_string(config.join("kabacusrc")).unwrap_or_default();
    assert!(!settings.contains("refresh"), "{settings}");

    // The next start signs in from the secret store, without the browser.
    let _app = launch_with(&server.socket_path, &config, &env);
    drive(&["resume"]).await;
    assert_eq!(provider.tokens_issued(), 2, "one refresh, no new sign-in");
    let _ = std::fs::remove_dir_all(&config);
}

/// The real Secret Service (KWallet, as on Plasma) keeps the refresh token
/// for the next connection, per server and provider, and forgets it on
/// sign-out.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Secret Service; run scripts/gui-e2e"]
async fn the_secret_service_keeps_the_refresh_token() {
    use kabacus_kde::secrets::{SecretAccount, SecretServiceStore, SecretStore};
    let account = SecretAccount {
        server: "unix:/run/kabacus-e2e.sock".into(),
        issuer: "https://provider.test".into(),
    };
    let other = SecretAccount {
        issuer: "https://other.test".into(),
        ..account.clone()
    };
    let store = SecretServiceStore;
    store.clear(&account).await.unwrap();

    store.save(&account, "first").await.unwrap();
    store.save(&account, "rotated").await.unwrap();

    let again = SecretServiceStore;
    assert_eq!(
        again.load(&account).await.unwrap().as_deref(),
        Some("rotated")
    );
    assert_eq!(again.load(&other).await.unwrap(), None);
    again.clear(&account).await.unwrap();
    assert_eq!(again.load(&account).await.unwrap(), None);
}

// A server with authentication off shows a visible warning.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn a_no_auth_server_shows_authentication_off() {
    let socket = socket_path("authoff");
    let server = spawn_server(|c| c.socket_path = socket.clone()).await;
    let _app = launch(&server.socket_path, &config_dir("authoff"));

    drive(&["auth-off"]).await;
}

// --- the new screens ----------------------------------

/// Books with a bank account, two funds, and a card whose reserve misses
/// one purchase (as tests/screens_e2e.rs).
const BOOKS: &str = "
account assets:bank:checking   ; ynab-account:Checking
account assets:bank:checking:insurance   ; ynab-envelope:Insurance
account assets:bank:checking:gifts   ; ynab-envelope:Gifts
account assets:bank:checking:card-visa
account liabilities:credit-card:visa   ; ynab-account:Visa
account expenses:dining   ; ynab-envelope:Dining

2026-09-01 Opening
    assets:bank:checking  2000.00 USD
    equity:opening

2026-09-10 Coffee Roasters
    expenses:dining  20.00 USD
    liabilities:credit-card:visa  -20.00 USD
";

async fn books(name: &str) -> kabacus_testkit::TestServer {
    let socket = socket_path(name);
    kabacus_testkit::spawn_server_with_journal(BOOKS, |c| c.socket_path = socket.clone()).await
}

/// A change from another client, committed while the app is open.
async fn change_elsewhere(server: &kabacus_testkit::TestServer, ops: Vec<BatchOpBody>) {
    let state = &server.state;
    let head = state
        .store
        .revision(state.ledger)
        .await
        .unwrap()
        .to_string();
    KabacusClient::connect_unix(&server.socket_path)
        .batch(&head, ops, false)
        .await
        .unwrap();
}

async fn card(client: &KabacusClient) -> kabacus_kde::views::CardDetail {
    kabacus_kde::views::cards(client)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.card.card == "liabilities:credit-card:visa")
        .unwrap()
}

// Add, Card purchase, the preview (live beside the form) and Save, committed with its reserve lines.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn a_card_purchase_is_recorded_with_its_reserve_on_every_form_factor() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("purchase{i}")).await;
        let client = KabacusClient::connect_unix(&server.socket_path);
        let app = launch_sized(
            &server.socket_path,
            &config_dir(&format!("purchase{i}")),
            size,
        );
        drive(&["card-purchase", "Corner Market", "42.18", "Visa", "Dining"]).await;
        drop(app);
        let visa = card(&client).await;
        assert_eq!(visa.card.owed.exact, "62.18", "{size}: {visa:?}");
        assert_eq!(
            visa.card.reserve.exact, "42.18",
            "{size}: reserve lines written"
        );
    }
}

// The edit form opens with the transaction's own
// From and To, and keeps them when the pickers' account list is reloaded
// under it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn the_edit_form_keeps_its_accounts_when_the_pickers_reload() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("prefill{i}")).await;
        let config = config_dir(&format!("prefill{i}"));
        let sync = config.join("sync");
        let _app = launch_sized(&server.socket_path, &config, size);
        let driver = {
            let sync = sync.display().to_string();
            tokio::spawn(async move {
                drive(&[
                    "edit-prefill",
                    "Visa",
                    "Coffee Roasters",
                    "2026-09-10",
                    "Visa",
                    "Dining",
                    &sync,
                ])
                .await
            })
        };
        let ready = sync.with_extension("ready");
        for _ in 0..300 {
            if ready.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        // A new account for the pickers: another client sets money aside
        // into a new fund.
        let other = KabacusClient::connect_unix(&server.socket_path);
        let aside = kabacus_kde::writes::Draft::SetAside {
            date: "2026-09-12".into(),
            from: "assets:bank:checking".into(),
            to: "Books".into(),
            amount: "15.00".into(),
            description: String::new(),
        };
        let p = kabacus_kde::writes::preview(&other, &aside).await.unwrap();
        kabacus_kde::writes::commit(&other, &aside, &p.head)
            .await
            .unwrap();
        std::fs::write(sync.with_extension("go"), "").unwrap();
        driver.await.unwrap();
    }
}

// A change from another client shows once you navigate back to the
// register.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn a_change_from_elsewhere_shows_after_navigating() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("handedit{i}")).await;
        let config = config_dir(&format!("handedit{i}"));
        let sync = config.join("sync");
        let _app = launch_sized(&server.socket_path, &config, size);
        let driver = {
            let sync = sync.display().to_string();
            tokio::spawn(async move {
                drive(&[
                    "external-edit",
                    "Visa",
                    "Coffee Roasters",
                    "Roastery by hand",
                    &sync,
                ])
                .await
            })
        };
        let ready = sync.with_extension("ready");
        for _ in 0..300 {
            if ready.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let client = KabacusClient::connect_unix(&server.socket_path);
        let ops = client
            .list_transactions()
            .await
            .unwrap()
            .into_iter()
            .filter(|t| t.payee == "Coffee Roasters")
            .map(|t| BatchOpBody {
                tool: "edit_transaction".to_string(),
                args: serde_json::json!({
                    "id": t.id,
                    "date": t.date,
                    "description": "Roastery by hand",
                    "postings": t.postings.iter().map(|p| serde_json::json!({
                        "account": p.account,
                        "amount": p.amount.exact,
                        "status": p.status,
                    })).collect::<Vec<_>>(),
                }),
            })
            .collect();
        change_elsewhere(&server, ops).await;
        std::fs::write(sync.with_extension("go"), "").unwrap();
        driver.await.unwrap();
    }
}

// Add from a card's register presets that card.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn add_from_a_register_presets_its_account() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("regadd{i}")).await;
        let _app = launch_sized(
            &server.socket_path,
            &config_dir(&format!("regadd{i}")),
            size,
        );
        drive(&["register-add", "Visa", "Spent", "From", "Visa"]).await;
    }
}

// A transaction without an id opens in the editor,
// and saving gives it one.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn a_transaction_without_an_id_opens_in_the_editor() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("noid{i}")).await;
        let client = KabacusClient::connect_unix(&server.socket_path);
        let app = launch_sized(&server.socket_path, &config_dir(&format!("noid{i}")), size);
        drive(&[
            "edit",
            "Checking",
            "Opening",
            "2026-09-01",
            "2026-09-01",
            "Opening balance",
            "2000.00",
            "Checking",
        ])
        .await;
        drop(app);
        let opening = client
            .list_transactions()
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.payee == "Opening balance")
            .expect("edited in place");
        // The scratch books' own opening is #1.
        assert_eq!(opening.id, 2, "{size}: edited where it was");
    }
}

// Accounts' headings come before their accounts for screen
// readers; a search with no matches says so and clears; a tap on a row's
// chip opens the row.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn headings_searches_and_chips_behave() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("polish{i}")).await;
        let config = config_dir(&format!("polish{i}"));
        {
            let _app = launch_sized(&server.socket_path, &config, size);
            drive(&["heading-order", "Money", "Checking", "Cards", "Visa"]).await;
        }
        {
            let _app = launch_sized(&server.socket_path, &config, size);
            drive(&["empty-search", "Checking", "Opening"]).await;
        }
        {
            let _app = launch_sized(&server.socket_path, &config, size);
            drive(&["chip-tap", "Visa", "unreserved"]).await;
        }
    }
}

// An amount finer than the commodity shows its
// problem while typing.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn an_amount_finer_than_the_commodity_is_flagged_while_typing() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("precision{i}")).await;
        let _app = launch_sized(
            &server.socket_path,
            &config_dir(&format!("precision{i}")),
            size,
        );
        drive(&["amount-precision", "12.345"]).await;
    }
}

// The Overview: Cash on Hand explains itself behind
// its info button and opens into its sum, free money and reserves list
// only what holds money (Gifts is empty), an account opens its register
// and a reserve the set-aside form. KDE's menu bar is shown.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn the_overview_explains_its_figures_and_opens_its_rows() {
    let journal = format!(
        "{BOOKS}
2026-09-02 Insurance savings
    assets:bank:checking:insurance  300.00 USD
    assets:bank:checking  -300.00 USD
"
    );
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let name = format!("overview{i}");
        let socket = socket_path(&name);
        let server = kabacus_testkit::spawn_server_with_journal(&journal, |c| {
            c.socket_path = socket.clone()
        })
        .await;
        let _app = launch_sized(&server.socket_path, &config_dir(&name), size);
        drive(&["overview", "Checking", "Insurance", "Gifts"]).await;
    }
}

// Loans lists each loan (a credit card isn't one),
// and a loan opens its register.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn loans_lists_each_loan_and_opens_its_register() {
    let journal = format!(
        "{BOOKS}
account liabilities:loans:car   ; ynab-account:Auto Loan
account liabilities:mortgage:home   ; ynab-account:Home Mortgage

2026-09-01 Auto loan balance from Lakeside Credit Union
    liabilities:loans:car  -14250.00 USD
    equity:opening

2026-09-01 Mortgage balance from Harbor Federal
    liabilities:mortgage:home  -238400.00 USD
    equity:opening
"
    );
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let name = format!("loans{i}");
        let socket = socket_path(&name);
        let server = kabacus_testkit::spawn_server_with_journal(&journal, |c| {
            c.socket_path = socket.clone()
        })
        .await;
        let _app = launch_sized(&server.socket_path, &config_dir(&name), size);
        drive(&[
            "loans",
            "Home Mortgage",
            "Mortgage balance from Harbor Federal",
        ])
        .await;
    }
}

// Reconcile is on a bank account's register (not a
// destination of its own, and not on a spending account's) and checks the
// account against a statement.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn reconcile_is_on_an_accounts_register() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let name = format!("reconcile{i}");
        let server = books(&name).await;
        let _app = launch_sized(&server.socket_path, &config_dir(&name), size);
        drive(&["reconcile", "Checking", "2000.00", "Dining"]).await;
    }
}

// Setting money aside in a fund.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn setting_money_aside_from_the_funds_page() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("aside{i}")).await;
        let client = KabacusClient::connect_unix(&server.socket_path);
        let app = launch_sized(&server.socket_path, &config_dir(&format!("aside{i}")), size);
        drive(&["set-aside", "Gifts", "75.00"]).await;
        drop(app);
        let o = kabacus_kde::views::overview(&client).await.unwrap();
        let gifts = o.funds.iter().find(|f| f.name == "Gifts").unwrap();
        assert_eq!(gifts.amount.exact, "75.00", "{size}");
        assert_eq!(o.cash_on_hand.exact, "1925.00", "{size}");
    }
}

// An edit opened and saved without a change has
// nothing to save, so Save is off and nothing is committed.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn an_unchanged_edit_has_nothing_to_save() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("unchanged{i}")).await;
        let client = KabacusClient::connect_unix(&server.socket_path);
        let head = client.version().await.unwrap().head;
        let app = launch_sized(
            &server.socket_path,
            &config_dir(&format!("unchanged{i}")),
            size,
        );
        drive(&["unchanged", "Visa", "Coffee Roasters", "2026-09-10"]).await;
        drop(app);
        assert_eq!(client.version().await.unwrap().head, head, "{size}");
    }
}

// A recorded transaction is edited in place or
// voided, and an edit keeps its Date editable.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn a_transaction_is_edited_or_voided() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("editmodes{i}")).await;
        let _app = launch_sized(
            &server.socket_path,
            &config_dir(&format!("editmodes{i}")),
            size,
        );
        drive(&["edit-modes", "Visa", "Coffee Roasters", "2026-09-10"]).await;
    }
}

// Leaving a form with a change typed in, by a bottom
// tab or the drawer, asks first instead of dropping the change.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn leaving_an_edited_form_asks_first() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("discard{i}")).await;
        let client = KabacusClient::connect_unix(&server.socket_path);
        let head = client.version().await.unwrap().head;
        let app = launch_sized(
            &server.socket_path,
            &config_dir(&format!("discard{i}")),
            size,
        );
        drive(&[
            "discard-prompt",
            "Visa",
            "Coffee Roasters",
            "2026-09-10",
            "24.50",
        ])
        .await;
        drop(app);
        assert_eq!(
            client.version().await.unwrap().head,
            head,
            "{size}: discarded, nothing written"
        );
    }
}

// Set aside into a fund that doesn't exist yet makes
// the fund, in the account the money comes from. A name that can't be a
// fund shows its error under To, with To's label still beside its field.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn setting_money_aside_into_a_new_fund() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("newfund{i}")).await;
        let client = KabacusClient::connect_unix(&server.socket_path);
        let app = launch_sized(
            &server.socket_path,
            &config_dir(&format!("newfund{i}")),
            size,
        );
        drive(&["new-fund", "Summer Vacation", "80.00"]).await;
        drop(app);
        let o = kabacus_kde::views::overview(&client).await.unwrap();
        let fund = o.funds.iter().find(|f| f.name == "Summer Vacation");
        assert_eq!(
            fund.map(|f| f.amount.exact.as_str()),
            Some("80.00"),
            "{size}: {:?}",
            o.funds
        );
    }
}

// Setting aside more than is free is refused under the Amount field, and
// nothing is written.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn setting_aside_more_than_is_free_is_refused() {
    for (i, size) in [DESKTOP].into_iter().enumerate() {
        let server = books(&format!("overdraw{i}")).await;
        let client = KabacusClient::connect_unix(&server.socket_path);
        let head = client.version().await.unwrap().head;
        let app = launch_sized(
            &server.socket_path,
            &config_dir(&format!("overdraw{i}")),
            size,
        );
        drive(&["overdraw-refused", "Gifts", "2500.00"]).await;
        drop(app);
        assert_eq!(
            client.version().await.unwrap().head,
            head,
            "{size}: nothing written"
        );
    }
}

// Fixing a card's missing reserve lines in one batch, on the desktop.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn the_cards_page_fixes_missing_reserve_lines() {
    let server = books("fix").await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    assert!(!card(&client).await.card.covered);
    let _app = launch(&server.socket_path, &config_dir("fix"));
    drive(&["fix-reserves"]).await;
    assert!(card(&client).await.card.covered);
}

// The books change between the
// preview and Save; nothing is written over them.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn a_change_made_meanwhile_is_a_conflict_not_an_overwrite() {
    let server = books("conflict").await;
    let config = config_dir("conflict");
    let sync = config.join("sync");
    let _app = launch_sized(&server.socket_path, &config, DESKTOP);
    let driver = {
        let sync = sync.display().to_string();
        tokio::spawn(async move {
            drive(&[
                "conflict",
                "Corner Market",
                "10.00",
                "Visa",
                "Dining",
                &sync,
            ])
            .await
        })
    };
    // Once the app shows its review, an MCP client changes the books.
    let ready = sync.with_extension("ready");
    for _ in 0..300 {
        if ready.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let other = KabacusClient::connect_unix(&server.socket_path).with_client_hint("mcp");
    let draft = kabacus_kde::writes::Draft::SetAside {
        date: "2026-10-01".into(),
        from: "assets:bank:checking".into(),
        to: "assets:bank:checking:gifts".into(),
        amount: "5.00".into(),
        description: String::new(),
    };
    let p = kabacus_kde::writes::preview(&other, &draft).await.unwrap();
    kabacus_kde::writes::commit(&other, &draft, &p.head)
        .await
        .unwrap();
    std::fs::write(sync.with_extension("go"), "").unwrap();
    driver.await.unwrap();

    let client = KabacusClient::connect_unix(&server.socket_path);
    assert_eq!(
        card(&client).await.card.owed.exact,
        "20.00",
        "the purchase wasn't written"
    );
}

// The offline banner: writes are off while the
// server can't be reached.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn offline_the_app_turns_writes_off() {
    let _app = launch_sized(
        &socket_path("offline-missing"),
        &config_dir("offline"),
        DESKTOP,
    );
    drive(&["offline"]).await;
}

/// The app's catalogs (po/), compiled for KI18n by scripts/gui-e2e
/// (`xtask i18n-build`) before the tests start: cargo holds the build
/// directory while tests run, so a test can't run xtask itself. Returns
/// the `share` directory, for `XDG_DATA_DIRS`.
fn catalogs() -> PathBuf {
    let share = repo_root().join("target/gui-e2e/share");
    assert!(
        share.join("locale/eo/LC_MESSAGES/kabacus.mo").exists(),
        "no catalogs in {}: run scripts/gui-e2e, which builds them",
        share.display()
    );
    share
}

// In Esperanto, Vietnamese and Hebrew: what QML
// shows (KI18n's i18n()) and what the Rust side words (the i18n! macros,
// through KI18n too) both come from the catalogs, the books' own names
// don't change, and the layout runs the way the language is written.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn the_app_speaks_the_users_language_on_every_form_factor() {
    let share = catalogs();
    let data_dirs = format!(
        "{}:{}",
        share.display(),
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
    );
    // Esperanto and Vietnamese, as asked for, and Hebrew for a language
    // written right to left: the whole app mirrors for it.
    let languages = [
        (
            "eo",
            "eo.UTF-8",
            "ltr",
            ["Superrigardo", "Elspezebla mono", "Transakcioj", "Mono"],
        ),
        (
            "vi",
            "vi_VN.UTF-8",
            "ltr",
            ["Tổng quan", "Tiền có thể tiêu", "Giao dịch", "Tiền"],
        ),
        (
            "he",
            "he_IL.UTF-8",
            "rtl",
            ["סקירה", "כסף זמין", "תנועות", "כסף"],
        ),
    ];
    for (language, locale, direction, [overview, cash, transactions, money]) in languages {
        for (i, size) in [DESKTOP].into_iter().enumerate() {
            let name = format!("language-{language}{i}");
            let server = books(&name).await;
            let _app = launch_full(
                &server.socket_path,
                &config_dir(&name),
                &[
                    // A real locale (the gui container has them all): with
                    // LC_MESSAGES at "C", gettext ignores LANGUAGE.
                    ("LC_ALL", locale),
                    ("LANG", locale),
                    ("LANGUAGE", language),
                    ("XDG_DATA_DIRS", &data_dirs),
                ],
                size,
            );
            drive(&[
                "language",
                overview,
                cash,
                transactions,
                money,
                "Checking",
                direction,
            ])
            .await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the gui build and a display; run scripts/gui-e2e"]
async fn desktop_split_postings_can_be_edited() {
    let socket = socket_path("edit-split");
    let journal = format!("{BOOKS}\n2026-09-12 Harbor Cafe\n    expenses:dining  12.00 USD\n    expenses:dining  8.00 USD\n    assets:bank:checking  -20.00 USD\n");
    let server =
        kabacus_testkit::spawn_server_with_journal(&journal, |c| c.socket_path = socket.clone())
            .await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let before = client
        .list_transactions()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.payee == "Harbor Cafe")
        .unwrap();
    let _app = launch(&server.socket_path, &config_dir("edit-split"));
    drive(&["edit-split", "Checking", "Harbor Cafe", "2026-09-12"]).await;
    let after = client
        .list_transactions()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == before.id)
        .unwrap();
    assert_eq!(
        after
            .postings
            .iter()
            .map(|p| p.amount.exact.as_str())
            .collect::<Vec<_>>(),
        vec!["15.00", "5.00", "-20.00"]
    );
}
