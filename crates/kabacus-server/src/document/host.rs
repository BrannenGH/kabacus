// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Snapshot hosts. Storage versions are independent of ledger revisions.

use anyhow::{bail, Context};
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::time::Duration;

pub enum Host {
    Local {
        path: PathBuf,
        _lock: File,
    },
    WebDav {
        url: reqwest::Url,
        http: reqwest::Client,
        token: Option<String>,
    },
}

#[derive(Clone)]
pub struct Snapshot {
    pub bytes: Vec<u8>,
    pub version: String,
}

impl Host {
    pub fn local(path: PathBuf) -> anyhow::Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let parent = parent.canonicalize()?;
        let path = parent.join(path.file_name().context("a document needs a file name")?);
        let lock_path = path.with_file_name(format!(
            "{}.lock",
            path.file_name().unwrap().to_string_lossy()
        ));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.try_lock()
            .context("this document already has a local session; connect to its socket")?;
        Ok(Self::Local { path, _lock: lock })
    }

    pub fn webdav(url: &str, token: Option<String>) -> anyhow::Result<Self> {
        let url = reqwest::Url::parse(url)?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("a document host must be an HTTPS URL without credentials, query or fragment");
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self::WebDav { url, http, token })
    }

    pub fn location(&self) -> String {
        match self {
            Self::Local { path, .. } => path.display().to_string(),
            Self::WebDav { url, .. } => url.to_string(),
        }
    }

    pub async fn read(&self, max: usize) -> anyhow::Result<Snapshot> {
        match self {
            Self::Local { path, .. } => {
                if std::fs::metadata(path)?.len() > max as u64 {
                    bail!("the document exceeds the size limit");
                }
                let bytes = std::fs::read(path)?;
                if bytes.len() > max {
                    bail!("the document exceeds the size limit");
                }
                Ok(Snapshot {
                    bytes,
                    version: String::new(),
                })
            }
            Self::WebDav { url, http, token } => {
                let mut request = http.get(url.clone());
                if let Some(token) = token {
                    request = request.bearer_auth(token);
                }
                let mut response = request.send().await?.error_for_status()?;
                let version = strong_etag(response.headers())?;
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    if chunk.len() > max.saturating_sub(bytes.len()) {
                        bail!("the document exceeds the size limit");
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(Snapshot { bytes, version })
            }
        }
    }

    pub async fn unchanged(&self, expected: &Snapshot, max: usize) -> anyhow::Result<()> {
        let current = self.read(max).await?;
        if current.version != expected.version || current.bytes != expected.bytes {
            bail!(
                "the external document changed; reopen it and reconcile the preserved working copy"
            );
        }
        Ok(())
    }

    pub async fn save(
        &self,
        expected: &Snapshot,
        bytes: Vec<u8>,
        max: usize,
    ) -> anyhow::Result<Snapshot> {
        if bytes.len() > max {
            bail!("the document exceeds the size limit");
        }
        match self {
            Self::Local { path, .. } => {
                // The stable sidecar lock coordinates Kabacus writers across atomic renames.
                self.unchanged(expected, max).await?;
                use std::io::Write;
                let mut scratch = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
                scratch.write_all(&bytes)?;
                scratch.as_file().sync_all()?;
                scratch.persist(path).map_err(|e| e.error)?;
                File::open(path.parent().unwrap())?.sync_all()?;
                Ok(Snapshot {
                    bytes,
                    version: String::new(),
                })
            }
            Self::WebDav { url, http, token } => {
                let mut request = http
                    .put(url.clone())
                    .header(reqwest::header::IF_MATCH, &expected.version)
                    .header(reqwest::header::CONTENT_TYPE, "application/vnd.kabacus")
                    .body(bytes.clone());
                if let Some(token) = token {
                    request = request.bearer_auth(token);
                }
                let response = request.send().await?;
                if response.status() == reqwest::StatusCode::PRECONDITION_FAILED
                    || response.status() == reqwest::StatusCode::CONFLICT
                {
                    bail!("the external document changed; the working copy was preserved");
                }
                let response = response.error_for_status()?;
                let version = strong_etag(response.headers())
                    .context("the host saved the file but returned no strong ETag; reopen before writing again")?;
                Ok(Snapshot { bytes, version })
            }
        }
    }
}

fn strong_etag(headers: &reqwest::header::HeaderMap) -> anyhow::Result<String> {
    let tag = headers
        .get(reqwest::header::ETAG)
        .context("the host must return a strong ETag")?
        .to_str()?;
    if tag.starts_with("W/") || !tag.starts_with('"') || !tag.ends_with('"') || tag.len() < 2 {
        bail!("the host must return a strong ETag");
    }
    Ok(tag.to_string())
}
