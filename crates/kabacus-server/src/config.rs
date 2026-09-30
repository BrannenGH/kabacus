// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Server configuration: TOML file plus CLI overrides. `container/server.toml`
//! is a commented example.

use serde::Deserialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The SQLite store's `.kabacus` file. Unset means
    /// `$KABACUS_DATABASE`, then `$XDG_DATA_HOME/kabacus/books.kabacus`.
    pub database: Option<PathBuf>,
    /// A PostgreSQL connection string, instead of `database`.
    pub database_url: Option<String>,
    /// Connections in the PostgreSQL pool.
    pub database_connections: usize,
    /// Which ledger in the store this server serves, by name: unset means
    /// the first. A store without it gets an empty one.
    pub ledger: Option<String>,
    /// The largest `.kabacus` file `POST /ledgers/import` takes, in bytes.
    pub import_max_bytes: usize,
    /// The store in use. Set by [`Config::resolve_store`], not in the
    /// config file.
    #[serde(skip)]
    pub store: Database,
    /// The unix socket kabacus-server itself listens on
    /// (`$XDG_RUNTIME_DIR/kabacus/server.sock`, mode 0600).
    pub socket_path: PathBuf,
    /// The first rewrite gate: on by default. `false` (or `--no-rewrite`)
    /// refuses rewrite/delete entirely; otherwise a caller still needs the
    /// `ledger-rewrite` role.
    pub allow_rewrite: bool,
    pub auth: AuthConfig,
    /// Off by default. When set, the server also listens on
    /// HTTPS for remote devices.
    pub remote: Option<RemoteConfig>,
    pub mcp: McpConfig,
    /// Built React bundle served at `/app/`; unset disables web hosting.
    pub web_dir: Option<PathBuf>,
    /// The ordered rule table that gives every account its kind
    /// (`[[account_kinds]]` with `pattern` and `kind`); the first match
    /// wins. Kept here, outside the books, and re-read on SIGHUP.
    pub account_kinds: Vec<kabacus_core::KindRule>,
    /// The reserve that mirrors each card, with `{card}` standing for the
    /// card account's last segment.
    pub card_reserve_account: String,
    /// The file this config was loaded from, re-read on SIGHUP.
    #[serde(skip)]
    pub config_file: Option<PathBuf>,
}

/// Where the books are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Database {
    Sqlite(PathBuf),
    Postgres(String),
}

impl Default for Database {
    fn default() -> Self {
        Database::Sqlite(PathBuf::new())
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            database: None,
            database_url: None,
            database_connections: 16,
            ledger: None,
            import_max_bytes: 256 * 1024 * 1024,
            store: Database::default(),
            socket_path: default_socket_path(),
            allow_rewrite: true,
            auth: AuthConfig::default(),
            remote: None,
            mcp: McpConfig::default(),
            web_dir: None,
            account_kinds: kabacus_core::default_kind_rules(),
            card_reserve_account: kabacus_core::DEFAULT_RESERVE_TEMPLATE.to_string(),
            config_file: None,
        }
    }
}

fn default_socket_path() -> PathBuf {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    runtime_dir.join("kabacus").join("server.sock")
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    Oauth,
    None,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TokenType {
    #[default]
    Jwt,
    Introspection,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AuthConfig {
    pub mode: AuthMode,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub token_type: TokenType,
    pub claims: ClaimsConfig,
    /// provider role/group/scope value -> Kabacus role name.
    pub role_map: std::collections::HashMap<String, String>,
    pub clients: ClientsConfig,
    /// Roles granted to the single local principal in `--auth=none`
    /// mode.
    pub no_auth_roles: Vec<String>,
    /// Clock-skew allowance, in seconds, for `exp`/`nbf`.
    pub clock_skew_seconds: u64,
    /// The server's own credentials for calling the
    /// provider's RFC 7662 introspection endpoint (opaque tokens only).
    pub introspection_client_id: Option<String>,
    pub introspection_client_secret: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClaimsConfig {
    pub username: String,
    pub client: String,
    pub session: Option<String>,
    pub roles: String,
}

impl Default for ClaimsConfig {
    fn default() -> Self {
        ClaimsConfig {
            username: "preferred_username".to_string(),
            client: "azp".to_string(),
            session: Some("sid".to_string()),
            roles: "roles".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ClientsConfig {
    pub app: Option<String>,
}

/// The MCP endpoint at `/mcp`. It needs no settings of its own; these
/// only tune what its RFC 9728 metadata tells MCP clients.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct McpConfig {
    /// The URL MCP clients reach `/mcp` at, when it isn't the remote
    /// listener's own address (e.g. behind a reverse proxy). Default: built
    /// from each request's `Host`.
    pub resource_url: Option<String>,
    /// The scopes MCP clients should ask the provider for, so their tokens
    /// carry the roles claim. Default: none advertised.
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteConfig {
    pub bind_addr: String,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

impl Config {
    pub fn load_from_str(toml_text: &str) -> anyhow::Result<Config> {
        Ok(toml::from_str(toml_text)?)
    }

    pub fn load_from_file(path: &std::path::Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)?;
        let mut config = Self::load_from_str(&text)?;
        config.config_file = Some(path.to_path_buf());
        Ok(config)
    }

    /// The kind rules and card reserve naming are usable.
    pub fn validate_kinds(&self) -> anyhow::Result<()> {
        kabacus_core::check_kind_rules(&self.account_kinds).map_err(anyhow::Error::msg)?;
        if !self.card_reserve_account.contains("{card}") {
            anyhow::bail!(
                "card_reserve_account {:?} must contain {{card}}, the card's last segment",
                self.card_reserve_account
            );
        }
        Ok(())
    }

    /// Work out the store from `database_url`, `database`,
    /// `$KABACUS_DATABASE` or the default file, in that order, unless it
    /// is already set.
    pub fn resolve_store(&mut self) -> anyhow::Result<()> {
        if self.store != Database::default() {
            return Ok(());
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        self.store = match (&self.database_url, &self.database) {
            (Some(url), None) => Database::Postgres(url.clone()),
            (Some(_), Some(_)) => {
                anyhow::bail!("set database or database_url, not both")
            }
            (None, Some(path)) => Database::Sqlite(expand_home(path, home.as_deref())),
            (None, None) => Database::Sqlite(default_database(
                std::env::var_os("KABACUS_DATABASE"),
                std::env::var_os("XDG_DATA_HOME"),
                home.as_deref(),
            )?),
        };
        Ok(())
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.store == Database::default() {
            anyhow::bail!("no books: set database (or database_url) in the config file");
        }
        if self.auth.mode == AuthMode::None && self.remote.is_some() {
            anyhow::bail!(
                "refusing to start: --auth=none can't be combined with the remote HTTPS listener"
            );
        }
        self.validate_kinds()
    }
}

fn default_database(
    explicit: Option<OsString>,
    data_home: Option<OsString>,
    home: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    if let Some(file) = explicit.filter(|f| !f.is_empty()) {
        return Ok(expand_home(Path::new(&file), home));
    }
    let data = match (data_home.filter(|d| !d.is_empty()), home) {
        (Some(dir), _) => PathBuf::from(dir),
        (None, Some(home)) => home.join(".local").join("share"),
        (None, None) => anyhow::bail!("no books: set database in the config file"),
    };
    Ok(data.join("kabacus").join("books.kabacus"))
}

fn expand_home(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_the_example_config() {
        let toml = r#"
            [auth]
            mode        = "oauth"
            issuer      = "https://id.example.lan/realms/home"
            audience    = "kabacus-server"
            token_type  = "jwt"

            [auth.claims]
            username    = "preferred_username"
            client      = "azp"
            session     = "sid"
            roles       = "resource_access.kabacus-server.roles"

            [auth.role_map]
            "ledger-read"    = "ledger-read"
            "ledger-write"   = "ledger-write"
            "ledger-rewrite" = "ledger-rewrite"

            [auth.clients]
            app = "kabacus-app"

            [mcp]
            resource_url = "https://books.example.lan:8443/mcp"
            scopes       = ["openid", "kabacus"]
        "#;
        let cfg = Config::load_from_str(toml).unwrap();
        assert_eq!(cfg.auth.mode, AuthMode::Oauth);
        assert_eq!(
            cfg.auth.issuer.as_deref(),
            Some("https://id.example.lan/realms/home")
        );
        assert_eq!(
            cfg.auth.claims.roles,
            "resource_access.kabacus-server.roles"
        );
        assert_eq!(
            cfg.auth.role_map.get("ledger-rewrite"),
            Some(&"ledger-rewrite".to_string())
        );
        assert_eq!(cfg.auth.clients.app.as_deref(), Some("kabacus-app"));
        assert_eq!(
            cfg.mcp.resource_url.as_deref(),
            Some("https://books.example.lan:8443/mcp")
        );
        assert_eq!(cfg.mcp.scopes, ["openid", "kabacus"]);
    }

    /// Rewrites are on unless the config (or `--no-rewrite`) turns them
    /// off; the `ledger-rewrite` role still gates every caller.
    #[test]
    fn rewrites_are_allowed_unless_turned_off() {
        assert!(Config::default().allow_rewrite);
        assert!(Config::load_from_str("").unwrap().allow_rewrite);
        assert!(
            !Config::load_from_str("allow_rewrite = false")
                .unwrap()
                .allow_rewrite
        );
    }
    #[test]
    fn account_kind_rules_come_from_the_config() {
        let cfg = Config::load_from_str("").unwrap();
        assert_eq!(cfg.account_kinds, kabacus_core::default_kind_rules());
        assert_eq!(cfg.account_kinds.len(), 13);
        assert_eq!(cfg.card_reserve_account, "assets:bank:checking:card-{card}");

        let cfg = Config::load_from_str(
            r#"
            card_reserve_account = "assets:bank:main:card-{card}"

            [[account_kinds]]
            pattern = "assets:bank:*:card-*"
            kind    = "debt-reserve"

            [[account_kinds]]
            pattern = "assets:wallet"
            kind    = "money"
            "#,
        )
        .unwrap();
        assert_eq!(
            cfg.account_kinds,
            [
                kabacus_core::KindRule::new(
                    "assets:bank:*:card-*",
                    kabacus_core::AccountKind::DebtReserve
                ),
                kabacus_core::KindRule::new("assets:wallet", kabacus_core::AccountKind::Money),
            ]
        );
        cfg.validate_kinds().unwrap();

        assert!(
            Config::load_from_str("[[account_kinds]]\npattern = \"x\"\nkind = \"envelope\"\n")
                .is_err()
        );
        let derived =
            Config::load_from_str("[[account_kinds]]\npattern = \"x\"\nkind = \"parent\"\n")
                .unwrap();
        assert!(derived.validate_kinds().is_err());
        let no_card = Config::load_from_str("card_reserve_account = \"assets:reserve\"\n").unwrap();
        assert!(no_card.validate_kinds().is_err());
    }
    #[test]
    fn refuses_no_auth_combined_with_remote_listener() {
        let mut cfg = Config {
            store: Database::Sqlite("/srv/books.kabacus".into()),
            auth: AuthConfig {
                mode: AuthMode::None,
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.remote = Some(RemoteConfig {
            bind_addr: "0.0.0.0:443".into(),
            cert_path: "cert.pem".into(),
            key_path: "key.pem".into(),
        });
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("--auth=none"), "{err}");
    }
    #[test]
    fn remote_listener_is_off_unless_configured() {
        assert!(Config::load_from_str("").unwrap().remote.is_none());
        let cfg = Config::load_from_str(
            "[remote]\nbind_addr = \"0.0.0.0:8443\"\ncert_path = \"c.pem\"\nkey_path = \"k.pem\"\n",
        )
        .unwrap();
        assert_eq!(cfg.remote.unwrap().bind_addr, "0.0.0.0:8443");
    }

    #[test]
    fn no_auth_without_remote_listener_is_fine() {
        let cfg = Config {
            store: Database::Sqlite("/srv/books.kabacus".into()),
            auth: AuthConfig {
                mode: AuthMode::None,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }
    #[test]
    fn the_database_defaults_to_kabacus_database_then_the_data_directory() {
        let home = Path::new("/home/someone");
        assert_eq!(
            default_database(Some("~/books/household.kabacus".into()), None, Some(home)).unwrap(),
            home.join("books/household.kabacus")
        );
        assert_eq!(
            default_database(None, Some("/data".into()), Some(home)).unwrap(),
            Path::new("/data/kabacus/books.kabacus")
        );
        assert_eq!(
            default_database(Some("".into()), None, Some(home)).unwrap(),
            home.join(".local/share/kabacus/books.kabacus")
        );
    }

    #[test]
    fn the_database_is_configurable() {
        let mut cfg = Config::load_from_str(
            "database = \"/srv/household.kabacus\"\nledger = \"Household\"\n",
        )
        .unwrap();
        cfg.resolve_store().unwrap();
        assert_eq!(cfg.store, Database::Sqlite("/srv/household.kabacus".into()));
        assert_eq!(cfg.ledger.as_deref(), Some("Household"));
        cfg.validate().unwrap();
        let mut cfg = Config::load_from_str(
            "database_url = \"postgresql://kabacus@db.example.lan/kabacus\"\n",
        )
        .unwrap();
        cfg.resolve_store().unwrap();
        assert_eq!(
            cfg.store,
            Database::Postgres("postgresql://kabacus@db.example.lan/kabacus".into())
        );
        let mut both = Config::load_from_str(
            "database = \"/srv/a.kabacus\"\ndatabase_url = \"postgresql:///kabacus\"\n",
        )
        .unwrap();
        assert!(both.resolve_store().is_err());
    }
}
