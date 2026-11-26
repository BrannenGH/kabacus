// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::bank_import::{apply_statement, ImportOptions};
use crate::books::{with_tags, WorkingCopy};
use crate::caller::{Caller, Permission};
use crate::error::Error;
use crate::service::require;
use crate::{ops, Service};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{Days, NaiveDate, Utc};
use kabacus_core::bank_import::simplefin::{link_key, Account, AccountSet};
use kabacus_core::bank_import::{clean_text, Decision, ImportError, SimplefinFeed};
use kabacus_core::{AccountKind, AccountName};
use kabacus_store::simplefin::{Access, Owner, RequestBudget};
use kabacus_store::BookStore;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use url::Url;

mod transport;
#[cfg(target_arch = "wasm32")]
pub use transport::Checkpoint;
pub use transport::{Http, Reply, Request, Response, Transport, BRIDGE_HOSTS};

const LINK: &str = "simplefin-account";
const HOST: &str = "simplefin-server";
const REVIEWED: &str = "simplefin-reviewed";
const LEGACY_FEED: &str = "simplefin-legacy-feed";
const AUTO_INTERVAL: i64 = 6 * 60 * 60;
const SNAPSHOT_TTL: i64 = 15 * 60;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub allowed_hosts: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_hosts: BRIDGE_HOSTS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Default)]
pub struct BankConnections {
    snapshots: Mutex<HashMap<Owner, Vec<Snapshot>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Window {
    since: NaiveDate,
    end: NaiveDate,
}

#[derive(Clone)]
struct Snapshot {
    id: String,
    access: Access,
    at: i64,
    window: Option<Window>,
    data: Arc<AccountSet>,
}

impl BankConnections {
    fn snapshot(
        &self,
        owner: Owner,
        access: &Access,
        now: i64,
        find: impl Fn(&Snapshot) -> bool,
    ) -> Option<Snapshot> {
        let mut cache = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|_, entries| {
            entries.retain(|entry| entry.at.saturating_add(SNAPSHOT_TTL) > now);
            !entries.is_empty()
        });
        cache
            .get(&owner)?
            .iter()
            .rev()
            .find(|entry| entry.access.same_claim(access) && find(entry))
            .cloned()
    }
    fn put(&self, owner: Owner, snapshot: Snapshot) {
        let mut cache = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        cache.entry(owner).or_default().push(snapshot);
    }
    fn clear(&self, owner: Owner) {
        self.snapshots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&owner);
    }
}

pub(crate) fn problem(code: &'static str, message: impl Into<String>) -> Error {
    Error::Import {
        code,
        message: message.into(),
    }
}

fn store_error(_: kabacus_store::StoreError) -> Error {
    problem(
        "simplefin_store",
        "The bank connection could not be read or saved",
    )
}

fn reconnect() -> Error {
    problem("simplefin_reconnect", "Reconnect to SimpleFIN to continue")
}

fn payment_required() -> Error {
    problem(
        "simplefin_payment",
        "Sorry, SimpleFIN is showing that your account with them is not paid. A paid SimpleFIN account is required",
    )
}

fn expired() -> Error {
    problem(
        "snapshot_expired",
        "Review this sync again to refresh the bank data",
    )
}

pub fn enabled(state: &Service) -> bool {
    state
        .settings
        .simplefin
        .as_ref()
        .is_none_or(|settings| settings.enabled)
}

fn allowlist(state: &Service) -> Option<&[String]> {
    state
        .settings
        .simplefin
        .as_ref()
        .map(|settings| settings.allowed_hosts.as_slice())
}

async fn owner(
    state: &Service,
    caller: &dyn Caller,
    permission: Permission,
) -> Result<Owner, Error> {
    if !enabled(state) {
        return Err(Error::NotFound("SimpleFIN".into()));
    }
    require(caller, permission)?;
    if !state.settings.simplefin_per_member {
        return Ok(Owner::Local);
    }
    let user = state.user_of(caller).await.map_err(|_| reconnect())?;
    Ok(Owner::Member(user))
}

async fn access(state: &Service, owner: Owner) -> Result<Access, Error> {
    state
        .store
        .simplefin_access(state.ledger, owner)
        .await
        .map_err(store_error)?
        .ok_or_else(reconnect)
}

fn claim_url(token: &str, allowlist: Option<&[String]>) -> Result<Url, Error> {
    let bytes = STANDARD
        .decode(token.trim())
        .map_err(|_| problem("simplefin_token", "Enter a SimpleFIN setup token"))?;
    let raw = std::str::from_utf8(&bytes)
        .map_err(|_| problem("simplefin_token", "Enter a SimpleFIN setup token"))?;
    let url = transport::validate_url(raw, allowlist)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(problem("simplefin_token", "Enter a SimpleFIN setup token"));
    }
    Ok(url)
}

fn credentials(access: &Access, allowlist: Option<&[String]>) -> Result<(Url, String), Error> {
    let mut url = transport::validate_url(access.url(), allowlist)?;
    let username = percent_encoding::percent_decode_str(url.username()).collect::<Vec<_>>();
    let password = percent_encoding::percent_decode_str(url.password().unwrap_or_default())
        .collect::<Vec<_>>();
    if username.is_empty() || username.contains(&b':') || password.is_empty() {
        return Err(problem(
            "simplefin_access",
            "The SimpleFIN server returned invalid access credentials",
        ));
    }
    let mut basic = username;
    basic.push(b':');
    basic.extend(password);
    url.set_password(None).map_err(|_| reconnect())?;
    url.set_username("").map_err(|_| reconnect())?;
    if url.query().is_some() {
        return Err(problem(
            "simplefin_access",
            "The SimpleFIN server returned an invalid access address",
        ));
    }
    Ok((url, format!("Basic {}", STANDARD.encode(basic))))
}

fn message(raw: &str, access: &Access, basic: &str) -> String {
    let mut safe = raw.replace(access.url(), "").replace(basic, "");
    if let Ok(url) = Url::parse(access.url()) {
        for secret in [url.username(), url.password().unwrap_or_default()] {
            if !secret.is_empty() {
                safe = safe.replace(secret, "");
                let decoded = percent_encoding::percent_decode_str(secret).decode_utf8_lossy();
                if !decoded.is_empty() {
                    safe = safe.replace(decoded.as_ref(), "");
                }
            }
        }
    }
    clean_text(&safe).chars().take(300).collect()
}

fn warning_values(snapshot: &Snapshot) -> Vec<Value> {
    snapshot.data.errlist.iter().map(|w| json!({"code": w.code, "message": w.msg, "conn_id": w.conn_id, "account_id": w.account_id}))
        .chain(snapshot.data.errors.iter().map(|warning| json!({"code":"gen.", "message": warning}))).collect()
}

async fn fetch(
    state: &Service,
    transport: &dyn Transport,
    owner: Owner,
    access: &Access,
    window: Option<Window>,
    refresh: bool,
    automatic: bool,
) -> Result<Snapshot, Error> {
    let now = Utc::now().timestamp();
    if !refresh {
        if let Some(snapshot) = state
            .bank_connections
            .snapshot(owner, access, now, |snapshot| {
                snapshot.window == window || window.is_none()
            })
        {
            return Ok(snapshot);
        }
    }
    let (mut url, authorization) = credentials(access, allowlist(state))?;
    let path = format!("{}/accounts", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.query_pairs_mut().append_pair("version", "2");
    if let Some(window) = window {
        let timestamp = |date: NaiveDate| {
            date.and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp()
                .to_string()
        };
        url.query_pairs_mut()
            .append_pair("start-date", &timestamp(window.since))
            .append_pair("end-date", &timestamp(window.end));
    } else {
        url.query_pairs_mut().append_pair("balances-only", "1");
    }
    match state
        .store
        .reserve_simplefin_request(
            state.ledger,
            owner,
            access,
            now,
            automatic.then_some(now - AUTO_INTERVAL),
        )
        .await
        .map_err(store_error)?
    {
        RequestBudget::Allowed { .. } => (),
        RequestBudget::Changed => return Err(reconnect()),
        RequestBudget::Deferred => {
            return Err(problem(
                "simplefin_deferred",
                "SimpleFIN was contacted recently",
            ))
        }
        RequestBudget::Exhausted { next_at } => {
            let date = chrono::DateTime::from_timestamp(next_at, 0)
                .map(|t| t.to_rfc3339())
                .unwrap_or_default();
            return Err(problem(
                "simplefin_budget",
                format!("SimpleFIN request limit reached. Try again after {date}"),
            ));
        }
    }
    #[cfg(target_arch = "wasm32")]
    transport.checkpoint().await?;
    let reply = transport
        .send(Request {
            url,
            authorization: Some(authorization.clone()),
            claim: false,
        })
        .await?;
    let wire = match reply.status {
        200 => reply.body,
        403 => return Err(reconnect()),
        402 => return Err(payment_required()),
        _ => {
            return Err(problem(
                "simplefin_network",
                "The SimpleFIN server could not return account data",
            ))
        }
    };
    let mut data: AccountSet = serde_json::from_slice(&wire).map_err(|_| {
        problem(
            "simplefin_version",
            "The SimpleFIN server does not support protocol version 2",
        )
    })?;
    let leaks = |text: &str| text.contains(access.url()) || text.contains(&authorization);
    if data.connections.iter().any(|c| leaks(&c.conn_id))
        || data.accounts.iter().any(|a| {
            leaks(&a.conn_id)
                || leaks(&a.id)
                || leaks(&a.currency)
                || leaks(&a.balance)
                || a.transactions
                    .iter()
                    .any(|t| leaks(&t.id) || leaks(&t.amount))
        })
    {
        return Err(problem(
            "simplefin_response",
            "The SimpleFIN server returned an invalid account response",
        ));
    }
    for warning in &mut data.errlist {
        #[cfg(not(target_arch = "wasm32"))]
        if warning.code.starts_with("gen.") {
            tracing::warn!(
                "SimpleFIN returned a general warning; details are in the import response"
            );
        }
        warning.msg = message(&warning.msg, access, &authorization);
        warning.code = message(&warning.code, access, &authorization);
        warning.conn_id = warning.conn_id.take().filter(|id| !leaks(id));
        warning.account_id = warning.account_id.take().filter(|id| !leaks(id));
    }
    if !data.errors.is_empty() {
        #[cfg(not(target_arch = "wasm32"))]
        tracing::warn!("SimpleFIN returned legacy warnings; details are in the import response");
    }
    for warning in &mut data.errors {
        *warning = message(warning, access, &authorization);
    }
    for connection in &mut data.connections {
        connection.name = message(&connection.name, access, &authorization);
    }
    for account in &mut data.accounts {
        account.name = message(&account.name, access, &authorization);
        for transaction in &mut account.transactions {
            transaction.description = message(&transaction.description, access, &authorization);
            if let Some(payee) = &mut transaction.payee {
                *payee = message(payee, access, &authorization);
            }
        }
    }
    if !crate::simplefin::access(state, owner)
        .await?
        .same_claim(access)
    {
        return Err(reconnect());
    }
    let snapshot = Snapshot {
        id: uuid::Uuid::new_v4().to_string(),
        access: access.clone(),
        at: now,
        window,
        data: Arc::new(data),
    };
    state.bank_connections.put(owner, snapshot.clone());
    Ok(snapshot)
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ConnectBody {
    pub token: String,
    #[serde(default)]
    pub replace: bool,
    #[serde(default)]
    pub dry_run: bool,
}

pub async fn connect(
    state: &Service,
    caller: &dyn Caller,
    transport: &dyn Transport,
    body: ConnectBody,
) -> Result<Value, Error> {
    let owner = owner(state, caller, Permission::Write).await?;
    let url = claim_url(&body.token, allowlist(state))?;
    let host = url.host_str().unwrap_or_default().to_string();
    let connected = state
        .store
        .simplefin_access(state.ledger, owner)
        .await
        .map_err(store_error)?
        .is_some();
    if body.dry_run {
        return Ok(
            json!({"host": host, "connected": connected, "local_file": state.settings.simplefin.is_none()}),
        );
    }
    if !body.replace && connected {
        return Err(problem(
            "simplefin_connected",
            "Confirm replacing your current SimpleFIN connection",
        ));
    }
    let result = transport
        .send(Request {
            url,
            authorization: None,
            claim: true,
        })
        .await?;
    if result.status == 402 {
        return Err(payment_required());
    }
    if result.status == 403 {
        return Err(problem("simplefin_token_used", "This SimpleFIN token was already used or is invalid. It may be compromised. Disable it at your SimpleFIN server and make a new one"));
    }
    if result.status != 200 {
        return Err(problem(
            "simplefin_claim",
            "The SimpleFIN server could not claim the setup token",
        ));
    }
    let url = transport::validate_url(
        std::str::from_utf8(&result.body).map_err(|_| reconnect())?,
        allowlist(state),
    )?;
    if url.host_str() != Some(host.as_str()) {
        return Err(problem(
            "simplefin_url",
            "The access address must stay on the SimpleFIN host you confirmed",
        ));
    }
    let access = Access::new(url.into(), Utc::now().timestamp());
    credentials(&access, allowlist(state))?;
    // Hosted books grant access through the configured OIDC role map.
    if let Owner::Member(user) = owner {
        let memberships = state.store.memberships(user).await.map_err(store_error)?;
        if !memberships
            .iter()
            .any(|(ledger, _)| *ledger == state.ledger)
        {
            state
                .store
                .set_membership(state.ledger, user, Some(kabacus_store::Role::Write))
                .await
                .map_err(store_error)?;
        }
    }
    state
        .store
        .set_simplefin_access(state.ledger, owner, &access)
        .await
        .map_err(store_error)?;
    state.bank_connections.clear(owner);
    let snapshot = fetch(state, transport, owner, &access, None, true, false).await?;
    account_list(state, &snapshot).await
}

pub async fn connection(state: &Service, caller: &dyn Caller) -> Result<Value, Error> {
    let owner = owner(state, caller, Permission::Read).await?;
    let Some(access) = state
        .store
        .simplefin_access(state.ledger, owner)
        .await
        .map_err(store_error)?
    else {
        return Ok(json!({"connected": false}));
    };
    let (url, _) = credentials(&access, allowlist(state))?;
    let now = Utc::now().timestamp();
    let snapshot = state
        .bank_connections
        .snapshot(owner, &access, now, |_| true);
    let connections = snapshot.as_ref().map(connection_list).unwrap_or_default();
    let warnings = snapshot.as_ref().map(warning_values).unwrap_or_default();
    Ok(
        json!({"connected": true, "automatic_sync": true, "host": url.host_str(), "claimed_at": access.claimed_at,
        "remaining": access.remaining(now), "connections": connections, "warnings": warnings}),
    )
}

pub async fn disconnect(state: &Service, caller: &dyn Caller) -> Result<Value, Error> {
    let owner = owner(state, caller, Permission::Write).await?;
    state
        .store
        .delete_simplefin_access(state.ledger, owner)
        .await
        .map_err(store_error)?;
    state.bank_connections.clear(owner);
    Ok(
        json!({"connected": false, "message": "Revoke the access at your SimpleFIN server to stop anyone using that credential"}),
    )
}

fn connection_list(snapshot: &Snapshot) -> Vec<Value> {
    snapshot
        .data
        .connections
        .iter()
        .map(|connection| json!({"conn_id": connection.conn_id, "name": connection.name}))
        .collect()
}

fn links(book: &kabacus_ledger::Book) -> BTreeMap<String, (String, Option<String>)> {
    book.accounts
        .iter()
        .filter_map(|(name, account)| {
            let tags = kabacus_core::declared_tags(&account.tags);
            let key = tags.iter().find(|(key, _)| key == LINK)?.1.clone();
            let host = tags
                .into_iter()
                .find(|(key, _)| key == HOST)
                .map(|(_, host)| host);
            Some((name.clone(), (key, host)))
        })
        .collect()
}

pub(crate) async fn account_links(
    state: &Service,
    caller: &dyn Caller,
    book: &kabacus_ledger::Book,
) -> BTreeMap<String, Option<bool>> {
    let links = links(book);
    if links.is_empty() {
        return BTreeMap::new();
    }
    let owned = if let Ok(owner) = owner(state, caller, Permission::Read).await {
        match state.store.simplefin_access(state.ledger, owner).await {
            Ok(Some(access)) => {
                state
                    .bank_connections
                    .snapshot(owner, &access, Utc::now().timestamp(), |_| true)
            }
            _ => {
                return links
                    .into_keys()
                    .map(|account| (account, Some(false)))
                    .collect()
            }
        }
    } else {
        None
    };
    let domain = owned
        .as_ref()
        .and_then(|snapshot| Url::parse(snapshot.access.url()).ok());
    links
        .into_iter()
        .map(|(name, (key, host))| {
            let served = owned
                .as_ref()
                .zip(domain.as_ref())
                .map(|(snapshot, domain)| {
                    snapshot.data.accounts.iter().any(|remote| {
                        matches_link(&key, &host, remote, domain.host_str().unwrap_or_default())
                    })
                });
            (name, served)
        })
        .collect()
}

fn matches_link(key: &str, host: &Option<String>, remote: &Account, domain: &str) -> bool {
    host.as_ref().is_none_or(|host| host == domain) && key == remote.key()
}

fn feed(wc: &WorkingCopy, target: &str, domain: &str, key: &str) -> SimplefinFeed {
    let identity = format!("{domain}/{key}");
    let accept_legacy = wc.book.accounts.get(target).is_none_or(|account| {
        kabacus_core::declared_tags(&account.tags)
            .iter()
            .find(|(name, _)| name == LEGACY_FEED)
            .is_none_or(|(_, value)| value == &identity || value.strip_prefix('/') == Some(key))
    });
    SimplefinFeed {
        key: identity,
        accept_legacy,
    }
}

async fn account_list(state: &Service, snapshot: &Snapshot) -> Result<Value, Error> {
    let books = state.books_snapshot().await.map_err(Error::Internal)?;
    let links = links(&books.book);
    let (url, _) = credentials(&snapshot.access, allowlist(state))?;
    let domain = url.host_str().unwrap_or_default();
    let accounts: Vec<_> = snapshot.data.accounts.iter().map(|remote| {
        let linked = links.iter().find(|(_, (key, host))| matches_link(key, host, remote, domain)).map(|(account, _)| account);
        json!({"id": remote.id, "conn_id": remote.conn_id, "name": remote.name, "currency": remote.currency,
            "balance": remote.balance, "balance_date": remote.balance_date, "linked": linked})
    }).collect();
    let other: Vec<_> = links
        .iter()
        .filter(|(_, (key, host))| {
            !snapshot
                .data
                .accounts
                .iter()
                .any(|remote| matches_link(key, host, remote, domain))
        })
        .map(|(account, _)| account)
        .collect();
    Ok(
        json!({"connected": true, "host": domain, "connections": connection_list(snapshot), "accounts": accounts,
        "other_links": other, "warnings": warning_values(snapshot)}),
    )
}

#[derive(Default, Deserialize, Serialize, JsonSchema)]
pub struct ListAccounts {
    #[serde(default)]
    pub refresh: bool,
}

pub async fn list_accounts(
    state: &Service,
    caller: &dyn Caller,
    transport: &dyn Transport,
    refresh: bool,
) -> Result<Value, Error> {
    let owner = owner(state, caller, Permission::Read).await?;
    let access = access(state, owner).await?;
    let snapshot = fetch(state, transport, owner, &access, None, refresh, false).await?;
    account_list(state, &snapshot).await
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct RemoteAccount {
    pub conn_id: String,
    pub id: String,
}

#[derive(Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum NewAccountKind {
    Money,
    RevolvingDebt,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct LinkBody {
    /// Null removes the link.
    pub remote: Option<RemoteAccount>,
    /// Declare a new account of this kind in the same revision as the link.
    pub create: Option<NewAccountKind>,
}

#[derive(Deserialize, JsonSchema)]
pub struct LinkParams {
    pub head: String,
    pub account: String,
    #[serde(flatten)]
    pub body: LinkBody,
}

pub async fn link(
    state: &Service,
    caller: &dyn Caller,
    transport: &dyn Transport,
    target: &str,
    head: &str,
    body: LinkBody,
) -> Result<Value, Error> {
    let owner = owner(state, caller, Permission::Write).await?;
    let mut wc = WorkingCopy::load(state, head).await?;
    let new_kind = body.create.map(|kind| match kind {
        NewAccountKind::Money => AccountKind::Money,
        NewAccountKind::RevolvingDebt => AccountKind::RevolvingDebt,
    });
    if new_kind.is_some() {
        if body.remote.is_none() {
            return Err(problem(
                "simplefin_account",
                "Choose a SimpleFIN account to link to the new account",
            ));
        }
        ops::declare_account(&mut wc, target, &[])?;
    }
    let chart = wc.chart();
    let eligible = chart.get(target).is_some_and(|account| {
        !account.synthetic
            && matches!(
                account.kind,
                AccountKind::Money | AccountKind::RevolvingDebt
            )
            && new_kind.is_none_or(|kind| account.kind == kind)
    });
    if !eligible {
        return Err(ImportError::AccountKind(target.into()).into());
    }
    let original = wc
        .book
        .accounts
        .get(target)
        .map_or("", |account| account.tags.as_str());
    let mut tags = kabacus_core::declared_tags(original);
    let old_feed = tags.iter().find(|(name, _)| name == LINK).map(|(_, key)| {
        let domain = tags
            .iter()
            .find(|(name, _)| name == HOST)
            .map_or("", |(_, value)| value.as_str());
        format!("{domain}/{key}")
    });
    tags.retain(|(key, _)| key != LINK && key != HOST);
    let new_feed = if let Some(remote) = body.remote {
        let access = access(state, owner).await?;
        let snapshot = fetch(state, transport, owner, &access, None, false, false).await?;
        let key = link_key(&remote.conn_id, &remote.id);
        let remote = snapshot
            .data
            .accounts
            .iter()
            .find(|remote| remote.key() == key)
            .ok_or_else(|| {
                problem(
                    "simplefin_account",
                    "Choose an account from your SimpleFIN connection",
                )
            })?;
        let commodity = wc.commodity();
        if remote.currency != commodity {
            return Err(ImportError::CurrencyMismatch {
                reported: clean_text(&remote.currency),
                commodity,
            }
            .into());
        }
        let (url, _) = credentials(&access, allowlist(state))?;
        let domain = url.host_str().unwrap_or_default();
        if links(&wc.book)
            .iter()
            .any(|(name, (key, host))| name != target && matches_link(key, host, remote, domain))
        {
            return Err(problem(
                "simplefin_linked",
                "This SimpleFIN account is already linked to another account",
            ));
        }
        let reviewed = format!("{domain}/{key}");
        tags.retain(|(name, value)| name != REVIEWED || value.starts_with(&format!("{reviewed} ")));
        tags.push((LINK.into(), key));
        tags.push((HOST.into(), domain.into()));
        Some(reviewed)
    } else {
        tags.retain(|(key, _)| key != REVIEWED);
        None
    };
    if let Some(old) = old_feed.filter(|old| Some(old) != new_feed.as_ref()) {
        // Older imports have no feed tag, so keep the mapping they belonged to.
        if !tags.iter().any(|(name, _)| name == LEGACY_FEED) {
            tags.push((LEGACY_FEED.into(), old));
        }
    }
    let text = with_tags(original, &tags);
    let changed = original != text;
    if new_kind == Some(AccountKind::RevolvingDebt) {
        let card = AccountName::new(target);
        let reserve = chart.reserve_for(&card);
        let valid_home = reserve.as_str().rsplit_once(':').is_some_and(|(home, _)| {
            wc.is_declared(home)
                && chart
                    .get(home)
                    .is_some_and(|account| account.kind == AccountKind::Money && !account.synthetic)
        });
        if chart.kind_of(&reserve) != AccountKind::DebtReserve || !valid_home {
            return Err(problem(
                "simplefin_reserve",
                "Configure a card reserve under a declared bank account before creating this card",
            ));
        }
        if chart.accounts.iter().any(|account| {
            account.kind == AccountKind::RevolvingDebt
                && account.account != card
                && chart.reserve_for(&account.account) == reserve
        }) {
            return Err(problem(
                "simplefin_reserve",
                "A different credit card already uses this reserve. Choose another name",
            ));
        }
        if !wc.is_declared(reserve.as_str()) {
            ops::declare_account(&mut wc, reserve.as_str(), &[])?;
        }
    }
    if changed {
        wc.declare(target, &text);
        ops::check(&wc)?;
        let who = ops::who(
            state,
            caller,
            "simplefin_link",
            &format!("Change the SimpleFIN link for {target}"),
        )
        .await?;
        let revision = ops::commit(state, wc.changes(), &who, wc.revision).await?;
        ops::remember(state, &wc, revision).await;
        wc.head = revision.to_string();
    }
    Ok(
        json!({"head": wc.head, "account": target, "linked": tags.iter().any(|(key, _)| key == LINK)}),
    )
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct SyncBody {
    /// Automatically import new rows on mappings that have completed a manual import.
    #[serde(default)]
    pub automatic: bool,
    pub dry_run: Option<bool>,
    pub snapshot: Option<String>,
    pub since: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    #[serde(default)]
    pub starts: BTreeMap<String, NaiveDate>,
    #[serde(default)]
    pub decisions: BTreeMap<String, Vec<Decision>>,
    #[serde(default)]
    pub refresh: bool,
    pub limit: Option<usize>,
    #[serde(default)]
    pub cursors: BTreeMap<String, String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SyncParams {
    pub head: String,
    #[serde(flatten)]
    pub body: SyncBody,
}

fn dates(
    wc: &WorkingCopy,
    links: &BTreeMap<String, (String, Option<String>)>,
    body: &SyncBody,
    end: NaiveDate,
    domain: &str,
) -> BTreeMap<String, NaiveDate> {
    let fallback = end.checked_sub_days(Days::new(31)).unwrap_or(end);
    links
        .iter()
        .map(|(account, (key, _))| {
            let start = body
                .since
                .or_else(|| body.starts.get(account).copied())
                .unwrap_or_else(|| {
                    let feed = feed(wc, account, domain, key);
                    let account = AccountName::new(account);
                    let prefix = format!(
                        "{}/simplefin:",
                        kabacus_core::bank_import::encode_id(account.as_str())
                    );
                    let latest = wc
                        .book
                        .transactions
                        .iter()
                        .filter(|tx| {
                            feed.matches(tx)
                                && tx.tags.iter().any(|(name, value)| {
                                    name == "import-target" && value.starts_with(&prefix)
                                })
                        })
                        .map(|tx| tx.date)
                        .max();
                    let assertion = wc
                        .book
                        .transactions
                        .iter()
                        .filter(|tx| {
                            tx.postings
                                .iter()
                                .any(|p| p.account == account && p.balance_assertion.is_some())
                        })
                        .map(|tx| tx.date)
                        .max();
                    latest
                        .and_then(|latest| latest.checked_sub_days(Days::new(10)))
                        .or_else(|| assertion.and_then(|assertion| assertion.succ_opt()))
                        .unwrap_or(fallback)
                });
            (account.clone(), start.min(end))
        })
        .collect()
}

pub async fn sync(
    state: &Service,
    caller: &dyn Caller,
    transport: &dyn Transport,
    head: &str,
    body: SyncBody,
) -> Result<Value, Error> {
    let owner = owner(state, caller, Permission::Write).await?;
    let access = access(state, owner).await?;
    let mut wc = WorkingCopy::load(state, head).await?;
    let automatic = body.automatic;
    if automatic
        && (body.snapshot.is_some()
            || body.since.is_some()
            || body.end_date.is_some()
            || !body.starts.is_empty()
            || !body.decisions.is_empty()
            || !body.cursors.is_empty()
            || body.dry_run.is_some()
            || body.refresh)
    {
        return Err(problem(
            "simplefin_automatic",
            "Use automatic sync without manual review options",
        ));
    }
    let dry_run = !automatic && body.dry_run.unwrap_or(true);
    let (url, _) = credentials(&access, allowlist(state))?;
    let domain = url.host_str().unwrap_or_default();
    let mut links = links(&wc.book);
    let mut cutovers = BTreeMap::new();
    if automatic {
        links.retain(|target, (key, host)| {
            if host.as_deref().is_some_and(|host| host != domain) {
                return false;
            }
            let prefix = format!("{domain}/{key} ");
            let cutover = wc.book.accounts.get(target).and_then(|account| {
                kabacus_core::declared_tags(&account.tags)
                    .iter()
                    .find_map(|(name, value)| {
                        if name != REVIEWED {
                            return None;
                        }
                        value.strip_prefix(&prefix)?.parse::<NaiveDate>().ok()
                    })
            });
            if let Some(date) = cutover {
                cutovers.insert(target.clone(), date);
            }
            cutover.is_some()
        });
        if links.is_empty() {
            return Ok(json!({"head": wc.head, "skipped": "needs_review"}));
        }
    }
    let now = Utc::now();
    let existing = if let Some(id) = &body.snapshot {
        Some(
            state
                .bank_connections
                .snapshot(owner, &access, now.timestamp(), |snapshot| {
                    &snapshot.id == id && snapshot.window.is_some()
                })
                .ok_or_else(expired)?,
        )
    } else if !dry_run && !automatic {
        return Err(expired());
    } else {
        None
    };
    let end = body
        .end_date
        .or_else(|| existing.as_ref().and_then(|s| s.window.map(|w| w.end)))
        .or_else(|| now.date_naive().succ_opt())
        .unwrap_or(now.date_naive());
    let previous = existing.clone().or_else(|| {
        state
            .bank_connections
            .snapshot(owner, &access, now.timestamp(), |_| true)
    });
    let owned: BTreeMap<_, _> = links
        .iter()
        .filter(|(_, (key, host))| {
            previous.as_ref().is_none_or(|s| {
                s.data
                    .accounts
                    .iter()
                    .any(|remote| matches_link(key, host, remote, domain))
            })
        })
        .map(|(name, link)| (name.clone(), link.clone()))
        .collect();
    let mut starts = dates(&wc, &owned, &body, end, domain);
    let earliest = end.checked_sub_days(Days::new(90)).unwrap_or(end);
    if automatic {
        if owned.is_empty() {
            return Ok(json!({"head": wc.head, "skipped": "no_accounts"}));
        }
        for (account, date) in &mut starts {
            *date = (*date).max(earliest).max(cutovers[account].min(end));
        }
        starts.retain(|_, date| *date < end);
        if starts.is_empty() {
            return Ok(json!({"head": wc.head, "skipped": "no_new_days"}));
        }
        links.retain(|account, _| starts.contains_key(account));
    }
    let since = starts
        .values()
        .copied()
        .min()
        .or(body.since)
        .unwrap_or_else(|| end.checked_sub_days(Days::new(31)).unwrap_or(end));
    if since > end
        || end.signed_duration_since(since).num_days() > 90
        || body.since.is_some_and(|since| since > end)
    {
        return Err(problem(
            "simplefin_window",
            "Choose at most 90 days. Use a statement file for older transactions",
        ));
    }
    let window = Window { since, end };
    let snapshot = match existing {
        Some(snapshot) if snapshot.window == Some(window) => snapshot,
        Some(_) => return Err(expired()),
        None => match fetch(
            state,
            transport,
            owner,
            &access,
            Some(window),
            body.refresh || automatic,
            automatic,
        )
        .await
        {
            Err(error) if automatic && error.code() == "simplefin_deferred" => {
                return Ok(json!({"head": wc.head, "skipped": "recent_request"}));
            }
            result => result?,
        },
    };
    let starts = if automatic {
        starts
    } else {
        dates(&wc, &links, &body, end, domain)
    };
    let mut decisions = body.decisions;
    let mut results = Vec::new();
    let mut count = 0usize;
    let mut added = 0u64;
    let mut matched = 0u64;
    for (target, (key, host)) in links {
        let Some(remote) = snapshot
            .data
            .accounts
            .iter()
            .find(|remote| matches_link(&key, &host, remote, domain))
        else {
            results.push(json!({"account": target, "other_connection": true}));
            continue;
        };
        if snapshot
            .data
            .errlist
            .iter()
            .any(|warning| warning.blocks(remote))
        {
            results.push(json!({"account": target, "unavailable": true}));
            continue;
        }
        let (mut statement, pending) = remote.statement()?;
        statement.rows.retain(|row| row.date < end);
        count += statement.rows.len() + pending;
        if count > state.settings.statement_limits.max_rows {
            return Err(ImportError::TooLarge.into());
        }
        let options = ImportOptions {
            dry_run: Some(dry_run),
            require_unambiguous: automatic,
            since: starts.get(&target).copied(),
            limit: body.limit,
            cursor: body.cursors.get(&target).cloned(),
            decisions: decisions.remove(&target).unwrap_or_default(),
            ..Default::default()
        };
        let feed = feed(&wc, &target, domain, &key);
        let mut result = apply_statement(&mut wc, &target, statement, &options, Some(&feed))?;
        result["pending"] = json!(pending);
        result["since"] = json!(options.since);
        let new = result["counts"]["new"].as_u64().unwrap_or_default();
        let matches = result["counts"]["matched"].as_u64().unwrap_or_default();
        if !dry_run && !automatic && new + matches > 0 {
            let original = &wc.book.accounts[&target].tags;
            let mut tags = kabacus_core::declared_tags(original);
            let prefix = format!("{domain}/{key} ");
            let cutoff = tags
                .iter()
                .filter_map(|(name, value)| {
                    if name != REVIEWED {
                        return None;
                    }
                    value.strip_prefix(&prefix)?.parse::<NaiveDate>().ok()
                })
                .max()
                .unwrap_or(end)
                .max(end);
            tags.retain(|(key, _)| key != REVIEWED);
            tags.push((REVIEWED.into(), format!("{prefix}{cutoff}")));
            let updated = with_tags(original, &tags);
            if &updated != original {
                wc.declare(&target, &updated);
            }
        }
        added += new;
        matched += matches;
        results.push(result);
    }
    if let Some(choice) = decisions.into_values().flatten().next() {
        return Err(ImportError::Decision(choice.import_id).into());
    }
    ops::check(&wc)?;
    if !dry_run && (added + matched != 0) {
        if !crate::simplefin::access(state, owner)
            .await?
            .same_claim(&access)
        {
            return Err(expired());
        }
        let mut who = ops::who(
            state,
            caller,
            "import",
            &format!("import: {added} added, {matched} matched (SimpleFIN)"),
        )
        .await?;
        who.details["source"] =
            json!({"format":"simplefin", "from": since, "to": end, "automatic": automatic});
        let revision = ops::commit(state, wc.changes(), &who, wc.revision).await?;
        ops::remember(state, &wc, revision).await;
        wc.head = revision.to_string();
        for result in &mut results {
            if result.get("head").is_some() {
                result["head"] = json!(wc.head);
            }
        }
    }
    Ok(
        json!({"head": wc.head, "dry_run": dry_run, "snapshot": snapshot.id, "accounts": results,
        "counts":{"new":added,"matched":matched}, "warnings":warning_values(&snapshot)}),
    )
}

#[cfg(test)]
mod tests;
