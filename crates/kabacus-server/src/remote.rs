// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The optional remote HTTPS listener for phones and tablets.
//!
//! Off unless `[remote]` is configured. It serves exactly the same router as
//! the unix socket, `/mcp` included, so every endpoint keeps its own OAuth
//! and role checks;
//! `Config::validate` already refuses `--auth=none` together with this
//! listener. The certificate is an ordinary one that chains to
//! the system trust store or the user's private CA; clients don't pin it.

use crate::config::RemoteConfig;
use anyhow::{Context, Result};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;
use tokio::net::TcpListener;

const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Load the configured PEM certificate chain and private key.
pub fn tls_config(remote: &RemoteConfig) -> Result<Arc<rustls::ServerConfig>> {
    let certs = CertificateDer::pem_file_iter(&remote.cert_path)
        .with_context(|| format!("reading {}", remote.cert_path.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("parsing {}", remote.cert_path.display()))?;
    let key = PrivateKeyDer::from_pem_file(&remote.key_path)
        .with_context(|| format!("reading {}", remote.key_path.display()))?;
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .context("the remote certificate and key don't match")?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Accept TLS connections on `listener` and serve `app` on each, forever.
/// A failed handshake (an untrusting client, a port scan) only drops that
/// connection.
pub async fn serve(listener: TcpListener, tls: Arc<rustls::ServerConfig>, app: axum::Router) {
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    // So `/mcp` advertises an https:// resource URL here.
    let app = app.layer(axum::Extension(crate::mcp::ViaHttps));
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::warn!("remote listener: accept failed: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let service = TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            // A peer that connects and never finishes the handshake doesn't
            // get to hold this task open.
            let stream = match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await {
                Ok(Ok(stream)) => stream,
                Ok(Err(e)) => {
                    tracing::debug!("remote listener: TLS handshake with {peer} failed: {e}");
                    return;
                }
                Err(_) => {
                    tracing::debug!("remote listener: TLS handshake with {peer} timed out");
                    return;
                }
            };
            if let Err(e) = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                tracing::debug!("remote listener: connection from {peer} ended: {e}");
            }
        });
    }
}
