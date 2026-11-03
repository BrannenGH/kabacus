// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `kabacus-client`: the typed async client for kabacus-server's API,
//! used by the KDE app. The app never opens the books itself:
//! everything goes through here.

pub mod oauth;
pub mod types;

use reqwest::Method;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use types::*;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("request failed: {0}")]
    Request(String),
    /// Connected, but no answer within `REQUEST_TIMEOUT`.
    #[error("the server didn't answer in time")]
    Timeout,
    #[error("unauthorized")]
    Unauthorized,
    /// Getting a token from the OAuth provider failed.
    #[error("sign-in failed: {0}")]
    Auth(String),
    /// The provider refused to refresh the app's session, or the user
    /// signed out: sign in again.
    #[error("signed out ({0}); sign in again")]
    SessionExpired(String),
    #[error("missing role: {0}")]
    Forbidden(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("the books changed (expected {expected}, now {current}); refetch and retry")]
    Conflict { expected: String, current: String },
    #[error("invalid ({rule}): {message}")]
    Invalid {
        rule: String,
        message: String,
        /// Each failure of the books' check, as a sentence with its place.
        failures: Vec<String>,
        /// The server's `details`: for a failed balance assertion the
        /// asserting `transaction` and `posting`, and every failure.
        details: serde_json::Value,
    },
    #[error("server error: {0}")]
    Server(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Talks to kabacus-server over its local unix socket or its
/// remote HTTPS listener. Cheap to clone: clones share the
/// connection pool.
#[derive(Clone)]
pub struct KabacusClient {
    http: reqwest::Client,
    base_url: String,
    auth: Option<Arc<dyn oauth::TokenSource>>,
    /// Sent as `Kabacus-Client-Hint`, identifying this caller (`"app"`),
    /// for no-auth-mode attribution. In OAuth mode the token's
    /// own client claim is used instead.
    client_hint: Option<String>,
}

/// An unreachable server fails fast instead of hanging the caller. A read
/// (a report, a statement PDF) may take longer than connecting, but not
/// for ever. A write has no limit: one given up on may still land, and
/// trying it again could record it twice. The OAuth calls (`oauth/`) are
/// safe to repeat and take both limits.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT)
}

impl KabacusClient {
    #[cfg(unix)]
    pub fn connect_unix(socket_path: impl Into<PathBuf>) -> Self {
        KabacusClient {
            // The host is ignored: every request goes down the socket.
            http: builder()
                .unix_socket(socket_path.into())
                .build()
                .expect("a unix-socket client builds"),
            base_url: "http://localhost".to_string(),
            auth: None,
            client_hint: None,
        }
    }

    /// Connect to a remote server at `base_url`
    /// (`https://host:port`). The server's certificate must chain to the
    /// system trust store or, if given, the PEM `private_ca`; it is never
    /// pinned. Plain `http://` is refused.
    pub fn connect_https(
        base_url: impl Into<String>,
        private_ca: Option<&[u8]>,
    ) -> Result<Self, ClientError> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if !base_url.starts_with("https://") {
            return Err(ClientError::Request(format!(
                "remote server address must be https://, got {base_url}"
            )));
        }
        // System trust store (rustls-tls-native-roots) plus the private CA.
        let mut builder = builder().https_only(true);
        if let Some(pem) = private_ca {
            for cert in reqwest::Certificate::from_pem_bundle(pem)
                .map_err(|e| ClientError::Request(format!("private CA: {e}")))?
            {
                builder = builder.add_root_certificate(cert);
            }
        }
        let http = builder
            .build()
            .map_err(|e| ClientError::Request(e.to_string()))?;
        Ok(KabacusClient {
            http,
            base_url,
            auth: None,
            client_hint: None,
        })
    }

    pub fn with_auth_token(self, token: impl Into<String>) -> Self {
        self.with_token_source(Arc::new(oauth::StaticToken(token.into())))
    }

    /// Get the bearer token for each request from `source`, which fetches
    /// and refreshes it as needed.
    pub fn with_token_source(mut self, source: Arc<dyn oauth::TokenSource>) -> Self {
        self.auth = Some(source);
        self
    }

    pub fn with_client_hint(mut self, hint: impl Into<String>) -> Self {
        self.client_hint = Some(hint.into());
        self
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        if_match: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> Result<(u16, Vec<u8>, reqwest::header::HeaderMap), ClientError> {
        let read = method == Method::GET;
        // Logged without the query: it can hold account names and amounts.
        let what = format!("{method} {}", path.split('?').next().unwrap_or(path));
        let started = std::time::Instant::now();
        let mut builder = self
            .http
            .request(method, format!("{}{path}", self.base_url));
        if read {
            builder = builder.timeout(REQUEST_TIMEOUT);
        }
        if let Some(source) = &self.auth {
            let token = source.token().await?;
            builder = builder.bearer_auth(token);
        }
        if let Some(hint) = &self.client_hint {
            builder = builder.header("kabacus-client-hint", hint);
        }
        if let Some(im) = if_match {
            builder = builder.header("if-match", im);
        }
        if let Some(body) = body.filter(|b| !b.is_empty()) {
            builder = builder
                .header("content-type", "application/json")
                .body(body);
        }
        let response = builder.send().await.map_err(|e| {
            log::warn!(
                "{what}: failed after {} ms: {e}",
                started.elapsed().as_millis()
            );
            match e.is_timeout() {
                true => ClientError::Timeout,
                false => ClientError::Request(format!("{e:?}")),
            }
        })?;
        let status = response.status().as_u16();
        let ms = started.elapsed().as_millis();
        if status >= 500 {
            log::warn!("{what} -> {status} in {ms} ms");
        } else {
            log::debug!("{what} -> {status} in {ms} ms");
        }
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|e| ClientError::Request(e.to_string()))?
            .to_vec();
        Ok((status, body, headers))
    }

    fn error_from_status(status: u16, body: &[u8]) -> ClientError {
        let value: serde_json::Value =
            serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
        let details = value.get("details").cloned().unwrap_or_default();
        // The server puts specifics under `details`; older bodies had them
        // at the top level.
        let field = |key: &str| {
            details
                .get(key)
                .or_else(|| value.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let message = value
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let code = value
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        match status {
            401 => ClientError::Unauthorized,
            403 => ClientError::Forbidden({
                let role = field("role");
                if role.is_empty() {
                    "unknown".to_string()
                } else {
                    role
                }
            }),
            404 => ClientError::NotFound({
                let id = field("id");
                if id.is_empty() {
                    "unknown".to_string()
                } else {
                    id
                }
            }),
            409 if code == "document_conflict" => ClientError::Server(format!(
                "{message} Recovery directory: {}",
                field("recovery_directory")
            )),
            409 => ClientError::Conflict {
                expected: field("expected"),
                current: {
                    let current = field("current_head");
                    if current.is_empty() {
                        field("current")
                    } else {
                        current
                    }
                },
            },
            400 | 422 => ClientError::Invalid {
                rule: {
                    let rule = field("rule");
                    if rule.is_empty() {
                        code.to_string()
                    } else {
                        rule
                    }
                },
                message,
                failures: details["failures"]
                    .as_array()
                    .map(|failures| {
                        failures
                            .iter()
                            .filter_map(|f| f["message"].as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                details,
            },
            // A proxy in front of the server (a 502 or 504) often sends
            // no body: the status is then all there is to go on.
            _ if body.is_empty() => ClientError::Server(format!("HTTP {status}")),
            _ => ClientError::Server(format!("HTTP {status}: {}", String::from_utf8_lossy(body))),
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let (status, body, _) = self.request(Method::GET, path, None, None).await?;
        if status >= 400 {
            return Err(Self::error_from_status(status, &body));
        }
        Ok(serde_json::from_slice(&body)?)
    }

    async fn write_json<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        if_match: &str,
        body: &impl serde::Serialize,
    ) -> Result<T, ClientError> {
        let payload = serde_json::to_vec(body)?;
        let (status, resp_body, _) = self
            .request(method, path, Some(if_match), Some(payload))
            .await?;
        if status >= 400 {
            return Err(Self::error_from_status(status, &resp_body));
        }
        Ok(serde_json::from_slice(&resp_body)?)
    }

    /// `GET /version` — unauthenticated: a device only needs the
    /// server address to discover the issuer and app client id.
    pub async fn version(&self) -> Result<VersionView, ClientError> {
        self.get_json("/version").await
    }

    pub async fn commodity(&self) -> Result<CommodityView, ClientError> {
        self.get_json("/commodity").await
    }

    pub async fn list_transactions(&self) -> Result<Vec<TransactionView>, ClientError> {
        self.get_json("/transactions").await
    }

    pub async fn list_accounts(
        &self,
        filter: Option<&str>,
    ) -> Result<Vec<AccountView>, ClientError> {
        match filter {
            Some(f) => {
                self.get_json(&format!("/accounts?filter={}", urlencode(f)))
                    .await
            }
            None => self.get_json("/accounts").await,
        }
    }

    pub async fn set_account_type(
        &self,
        account: &str,
        body: &SetAccountTypeBody,
        if_match: &str,
    ) -> Result<WriteResultView, ClientError> {
        self.write_json(
            Method::PUT,
            &format!("/accounts/{}/type", urlencode(account)),
            if_match,
            body,
        )
        .await
    }

    /// Known payees, most recent first, each with the transaction
    /// to pre-fill from; `query` narrows them fuzzily.
    pub async fn payees(&self, query: Option<&str>) -> Result<Vec<PayeeView>, ClientError> {
        match query {
            Some(q) => self.get_json(&format!("/payees?q={}", urlencode(q))).await,
            None => self.get_json("/payees").await,
        }
    }

    pub async fn get_register(
        &self,
        account: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<RegisterLineView>, ClientError> {
        let mut params = vec![];
        if let Some(f) = from {
            params.push(format!("from={f}"));
        }
        if let Some(t) = to {
            params.push(format!("to={t}"));
        }
        let query = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        self.get_json(&format!("/register/{account}{query}")).await
    }

    pub async fn add_transaction(
        &self,
        if_match: &str,
        body: AddTransactionBody,
    ) -> Result<WriteResultView, ClientError> {
        self.write_json(Method::POST, "/transactions", if_match, &body)
            .await
    }

    pub async fn correct_transaction(
        &self,
        id: u64,
        if_match: &str,
        description: String,
        postings: Vec<DraftPostingBody>,
        dry_run: bool,
    ) -> Result<WriteResultView, ClientError> {
        let body = CorrectionBody {
            void: false,
            description: Some(description),
            postings: Some(postings),
            dry_run,
            today: None,
        };
        self.write_json(
            Method::POST,
            &format!("/transactions/{id}/corrections"),
            if_match,
            &body,
        )
        .await
    }

    pub async fn void_transaction(
        &self,
        id: u64,
        if_match: &str,
        dry_run: bool,
    ) -> Result<WriteResultView, ClientError> {
        let body = CorrectionBody {
            void: true,
            description: None,
            postings: None,
            dry_run,
            today: None,
        };
        self.write_json(
            Method::POST,
            &format!("/transactions/{id}/corrections"),
            if_match,
            &body,
        )
        .await
    }

    /// Edit a transaction in place (`PATCH /transactions/{id}`).
    pub async fn edit_transaction(
        &self,
        id: u64,
        if_match: &str,
        body: AddTransactionBody,
    ) -> Result<WriteResultView, ClientError> {
        self.write_json(
            Method::PATCH,
            &format!("/transactions/{id}"),
            if_match,
            &body,
        )
        .await
    }

    pub async fn set_status(
        &self,
        id: u64,
        if_match: &str,
        account: &str,
        status: &str,
        dry_run: bool,
    ) -> Result<WriteResultView, ClientError> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            account: &'a str,
            status: &'a str,
            dry_run: bool,
        }
        self.write_json(
            Method::PATCH,
            &format!("/transactions/{id}/status"),
            if_match,
            &Body {
                account,
                status,
                dry_run,
            },
        )
        .await
    }

    pub async fn history(&self) -> Result<Vec<CommitView>, ClientError> {
        self.get_json("/history").await
    }

    pub async fn history_diff(&self, rev: &str) -> Result<String, ClientError> {
        let (status, body, _) = self
            .request(Method::GET, &format!("/history/{rev}"), None, None)
            .await?;
        if status >= 400 {
            return Err(Self::error_from_status(status, &body));
        }
        Ok(String::from_utf8_lossy(&body).to_string())
    }

    pub async fn counterparties(&self) -> Result<Vec<CounterpartyView>, ClientError> {
        self.get_json("/counterparties").await
    }

    pub async fn statement(
        &self,
        name: &str,
        from: &str,
        to: &str,
        open_items: bool,
    ) -> Result<StatementView, ClientError> {
        self.get_json(&format!(
            "/counterparties/{}/statement?from={from}&to={to}&open_items={open_items}",
            urlencode(name)
        ))
        .await
    }

    /// The statement as a `"csv"` or `"pdf"` file, named by the
    /// server (`statement-<name>-<end-date>.<ext>`). `header` is the PDF
    /// letterhead; the statement date defaults to today.
    pub async fn export_statement(
        &self,
        name: &str,
        from: &str,
        to: &str,
        open_items: bool,
        format: &str,
        header: Option<&kabacus_core::StatementHeader>,
    ) -> Result<StatementFile, ClientError> {
        let mut path = format!(
            "/counterparties/{}/statement?from={from}&to={to}&open_items={open_items}&format={}",
            urlencode(name),
            urlencode(format)
        );
        if let Some(header) = header {
            path.push_str(&format!(
                "&sender_name={}&sender_address={}&statement_date={}",
                urlencode(&header.sender_name),
                urlencode(&header.sender_address.join("\n")),
                header.statement_date.format("%Y-%m-%d")
            ));
        }
        let (status, bytes, headers) = self.request(Method::GET, &path, None, None).await?;
        if status >= 400 {
            return Err(Self::error_from_status(status, &bytes));
        }
        let header_str = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string()
        };
        let filename = header_str("content-disposition")
            .split("filename=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap_or_default()
            .to_string();
        Ok(StatementFile {
            filename,
            content_type: header_str("content-type"),
            bytes,
        })
    }

    /// One transaction.
    pub async fn get_transaction(&self, id: u64) -> Result<TransactionView, ClientError> {
        self.get_json(&format!("/transactions/{id}")).await
    }

    /// Search transactions with any of its filters, as `key=value` pairs (values are
    /// encoded here). Always the plain list, never a page.
    pub async fn search(
        &self,
        params: &[(&str, &str)],
    ) -> Result<Vec<TransactionView>, ClientError> {
        let query = params
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencode(v)))
            .collect::<Vec<_>>()
            .join("&");
        self.get_json(&format!("/transactions?{query}")).await
    }

    /// Cash on Hand; `detail` is `summary` or `full`.
    pub async fn cash_on_hand(
        &self,
        compare_to: Option<&str>,
        detail: &str,
    ) -> Result<CashOnHandView, ClientError> {
        let mut path = format!("/cash-on-hand?detail={}", urlencode(detail));
        if let Some(day) = compare_to {
            path.push_str(&format!("&compare_to={}", urlencode(day)));
        }
        self.get_json(&path).await
    }

    /// Each card, or only `card`, with its reserve by source.
    pub async fn card_reserves(&self, card: Option<&str>) -> Result<CardReservesView, ClientError> {
        match card {
            Some(c) => {
                self.get_json(&format!("/cards/reserves?card={}", urlencode(c)))
                    .await
            }
            None => self.get_json("/cards/reserves").await,
        }
    }

    /// Set money aside in a fund or card reserve, release it, or move it.
    pub async fn allocate(
        &self,
        if_match: &str,
        body: &AllocateBody,
    ) -> Result<WriteResultView, ClientError> {
        self.write_json(Method::POST, "/allocations", if_match, body)
            .await
    }

    /// A card purchase with its reserve lines.
    pub async fn record_card_purchase(
        &self,
        if_match: &str,
        body: &CardPurchaseBody,
    ) -> Result<WriteResultView, ClientError> {
        self.write_json(Method::POST, "/cards/purchases", if_match, body)
            .await
    }

    /// Pay a card from its reserve.
    pub async fn pay_card(
        &self,
        if_match: &str,
        body: &CardPaymentBody,
    ) -> Result<WriteResultView, ClientError> {
        self.write_json(Method::POST, "/cards/payments", if_match, body)
            .await
    }

    /// Compare an account with a statement; `assert` writes the
    /// reconciled balance assertion when it matches.
    pub async fn reconcile(&self, body: &ReconcileBody) -> Result<serde_json::Value, ClientError> {
        let payload = serde_json::to_vec(body)?;
        let (status, resp, _) = self
            .request(Method::POST, "/reconcile", None, Some(payload))
            .await?;
        if status >= 400 {
            return Err(Self::error_from_status(status, &resp));
        }
        Ok(serde_json::from_slice(&resp)?)
    }

    /// Several writes checked together and committed as one.
    pub async fn batch(
        &self,
        head: &str,
        ops: Vec<BatchOpBody>,
        dry_run: bool,
    ) -> Result<serde_json::Value, ClientError> {
        let body = serde_json::json!({ "head": head, "dry_run": dry_run, "ops": ops });
        let payload = serde_json::to_vec(&body)?;
        let (status, resp, _) = self
            .request(Method::POST, "/batch", None, Some(payload))
            .await?;
        if status >= 400 {
            return Err(Self::error_from_status(status, &resp));
        }
        Ok(serde_json::from_slice(&resp)?)
    }

    /// A report (`income-statement`, `categories`, `cash-flow`,
    /// `cash-on-hand-history`, `balance-sheet`, `receivables`) with its
    /// query parameters.
    pub async fn report(
        &self,
        name: &str,
        params: &[(&str, &str)],
    ) -> Result<serde_json::Value, ClientError> {
        let query = params
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencode(v)))
            .collect::<Vec<_>>()
            .join("&");
        self.get_json(&format!("/reports/{name}?{query}")).await
    }
}

/// An exported statement.
#[derive(Debug, Clone)]
pub struct StatementFile {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Minimal query-parameter percent-encoding (no extra crate for this one
/// use): escapes everything but unreserved characters.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::urlencode;

    #[test]
    fn urlencode_escapes_spaces_and_special_characters() {
        assert_eq!(urlencode("grocery store"), "grocery%20store");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencode("simple"), "simple");
    }

    /// A proxy's empty-bodied 504 still says what happened in Details.
    #[test]
    fn a_server_error_names_its_status() {
        let e = super::KabacusClient::error_from_status(504, b"");
        assert_eq!(e.to_string(), "server error: HTTP 504");
        let e = super::KabacusClient::error_from_status(500, b"boom");
        assert_eq!(e.to_string(), "server error: HTTP 500: boom");
    }

    #[test]
    fn a_document_checkpoint_failure_is_not_a_stale_ledger_head() {
        let body = serde_json::json!({
            "code": "document_conflict",
            "message": "Changes are saved locally but could not be uploaded.",
            "details": {
                "committed_locally": true,
                "recovery_directory": "/tmp/household-recovery",
                "retry_write": false,
            }
        });
        let error = super::KabacusClient::error_from_status(409, &serde_json::to_vec(&body).unwrap());
        assert!(matches!(error, super::ClientError::Server(_)));
        assert!(error.to_string().contains("saved locally"));
        assert!(error.to_string().contains("/tmp/household-recovery"));
    }
}
