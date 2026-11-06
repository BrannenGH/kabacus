// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

pub use kabacus_service::import::*;

use kabacus_ledger::ImportOutcome;
use kabacus_store::{BookStore, LedgerId, Who};
use std::path::Path;

/// `kabacus-server import`: read the journal at `main` into `store`'s
/// ledger `ledger` (made if missing), which must be empty.
pub async fn import_from_files(
    store: &kabacus_store::AnyStore,
    ledger: Option<&str>,
    main: &Path,
) -> anyhow::Result<(LedgerId, kabacus_store::Revision, ImportOutcome)> {
    let outcome = kabacus_ledger::import(&kabacus_ledger::FsSource, main)?;
    let id = crate::state::find_or_create_ledger(store, ledger).await?;
    let user = store.local_user(crate::state::SERVER_USER).await?;
    let name = main
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let who = Who {
        user,
        actor: "import".to_string(),
        kind: "import".to_string(),
        summary: format!("import: {name}"),
        details: serde_json::json!({ "file": main.display().to_string() }),
    };
    let revision = store.import(id, &outcome.book, &who).await?;
    Ok((id, revision, outcome))
}
