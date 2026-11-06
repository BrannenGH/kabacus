// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The same persistence contract runs on native and browser SQLite.
use kabacus_ledger::{Book, MemorySource};
use kabacus_store::{BookStore, ChangeSet, SqliteStore, StoreError, Who};
use std::{collections::BTreeMap, path::Path};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test;
fn book() -> Book {
    let path = Path::new("/household/main.journal");
    let source=MemorySource(BTreeMap::from([(path.into(),"commodity USD1.00\naccount assets:checking\naccount expenses:groceries\naccount equity:opening\n\n2026-01-01 Opening\n    assets:checking  3218.46 USD\n    equity:opening  -3218.46 USD\n".into())]));
    kabacus_ledger::import(&source, path).unwrap().book
}
async fn seeded() -> (SqliteStore, i64, Who) {
    let store = SqliteStore::in_memory().unwrap();
    let user = store.local_user("Dana Okafor").await.unwrap();
    let ledger = store.create_ledger("Household", user).await.unwrap();
    let who = Who {
        user,
        actor: "app".into(),
        kind: "import".into(),
        summary: "Opening balances".into(),
        details: serde_json::json!({"user":"Dana Okafor"}),
    };
    store.import(ledger, &book(), &who).await.unwrap();
    (store, ledger, who)
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
async fn revisions_conflicts_and_history_are_identical() {
    let (store, ledger, who) = seeded().await;
    let before = store.load(ledger, None).await.unwrap();
    let mut after = before.book.clone();
    after.transactions[0].description = "Opening balance corrected".into();
    let revision = store
        .commit(
            ledger,
            before.revision,
            ChangeSet::between(&before.book, &after),
            &who,
        )
        .await
        .unwrap();
    assert_eq!(revision, before.revision + 1);
    let conflict = store
        .commit(
            ledger,
            before.revision,
            ChangeSet::between(&before.book, &after),
            &who,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(conflict,StoreError::Conflict{expected,current}if expected==before.revision&&current==revision)
    );
    assert_eq!(
        store
            .load(ledger, Some(before.revision))
            .await
            .unwrap()
            .book,
        before.book
    );
    assert_eq!(
        store.history(ledger, 10).await.unwrap()[0].user,
        "Dana Okafor"
    );
    assert_eq!(store.transaction_history(ledger, 1).await.unwrap().len(), 2);
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
async fn an_undeclared_posting_changes_nothing() {
    let (store, ledger, who) = seeded().await;
    let before = store.load(ledger, None).await.unwrap();
    let mut after = before.book.clone();
    after.transactions[0].postings[0].account = kabacus_core::AccountName::new("assets:savings");
    assert!(store
        .commit(
            ledger,
            before.revision,
            ChangeSet::between(&before.book, &after),
            &who
        )
        .await
        .is_err());
    assert_eq!(store.load(ledger, None).await.unwrap().book, before.book);
    assert_eq!(store.revision(ledger).await.unwrap(), before.revision);
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
async fn sqlite_images_preserve_every_revision_and_exact_amount() {
    let (store, ledger, _) = seeded().await;
    let expected = store.load(ledger, None).await.unwrap();
    let bytes = store.export_bytes(&[ledger]).await.unwrap();
    assert_eq!(&bytes[..16], b"SQLite format 3\0");
    let reopened = SqliteStore::from_bytes(&bytes).unwrap();
    let id = reopened.ledgers().await.unwrap()[0].id;
    let actual = reopened.load(id, None).await.unwrap();
    assert_eq!(actual.book, expected.book);
    assert_eq!(actual.revision, expected.revision);
    assert_eq!(
        reopened.history(id, 20).await.unwrap(),
        store.history(ledger, 20).await.unwrap()
    );
    assert_eq!(
        actual.book.transactions[0].postings[0]
            .amount
            .money
            .to_string(),
        "3218.46"
    );
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
async fn ledgers_and_memberships_stay_isolated() {
    let (store, ledger, who) = seeded().await;
    let other = store.create_ledger("Cabin", who.user).await.unwrap();
    assert!(store
        .load(other, None)
        .await
        .unwrap()
        .book
        .transactions
        .is_empty());
    assert_eq!(
        store
            .load(ledger, None)
            .await
            .unwrap()
            .book
            .transactions
            .len(),
        1
    );
    assert_eq!(store.memberships(who.user).await.unwrap().len(), 2);
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
async fn byte_import_validates_the_whole_file_before_writing() {
    let (store, ledger, who) = seeded().await;
    let count = store.ledgers().await.unwrap().len();
    assert!(store
        .import_bytes(b"This is not a SQLite file", who.user)
        .await
        .is_err());
    assert_eq!(store.ledgers().await.unwrap().len(), count);
    let bytes = store.export_bytes(&[ledger]).await.unwrap();
    let ids = store.import_bytes(&bytes, who.user).await.unwrap();
    assert_eq!(ids.len(), 1);
    assert_ne!(ids[0], ledger);
    assert_eq!(
        store.load(ids[0], None).await.unwrap().book,
        store.load(ledger, None).await.unwrap().book
    );
}
