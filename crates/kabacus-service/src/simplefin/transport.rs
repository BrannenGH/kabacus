// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::problem;
use crate::error::Error;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use url::{Host, Url};

pub const BRIDGE_HOSTS: &[&str] = &["bridge.simplefin.org", "beta-bridge.simplefin.org"];
const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;

pub struct Request {
    pub url: Url,
    pub authorization: Option<String>,
    pub claim: bool,
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
pub type Reply<'a> = Pin<Box<dyn Future<Output = Result<Response, Error>> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
pub type Reply<'a> = Pin<Box<dyn Future<Output = Result<Response, Error>> + 'a>>;

#[cfg(not(target_arch = "wasm32"))]
pub trait Transport: Send + Sync {
    fn send(&self, request: Request) -> Reply<'_>;
}

#[cfg(target_arch = "wasm32")]
pub type Checkpoint<'a> = Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>>;

#[cfg(target_arch = "wasm32")]
pub trait Transport {
    fn send(&self, request: Request) -> Reply<'_>;
    fn checkpoint(&self) -> Checkpoint<'_> {
        Box::pin(async { Ok(()) })
    }
}

pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_documentation()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && matches!(b, 18 | 19)))
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(mapped));
            }
            let segments = v6.segments();
            (segments[0] & 0xe000) == 0x2000
                && !(segments[0] == 0x2001 && matches!(segments[1], 0x0000 | 0x0002 | 0x0db8))
        }
    }
}

pub fn validate_url(text: &str, allowed: Option<&[String]>) -> Result<Url, Error> {
    let invalid = || {
        problem(
            "simplefin_url",
            "Use a public HTTPS SimpleFIN server allowed by these books",
        )
    };
    let mut url = Url::parse(text.trim()).map_err(|_| invalid())?;
    if url.scheme() != "https" || url.fragment().is_some() {
        return Err(invalid());
    }
    match url.host() {
        Some(Host::Domain(domain))
            if domain.contains('.')
                && !domain.ends_with(['.'])
                && ![".localhost", ".local", ".internal", ".invalid", ".test"]
                    .iter()
                    .any(|suffix| domain.ends_with(suffix)) => {}
        Some(Host::Ipv4(ip)) if public_ip(IpAddr::V4(ip)) => (),
        Some(Host::Ipv6(ip)) if public_ip(IpAddr::V6(ip)) => (),
        _ => return Err(invalid()),
    }
    let host = url.host_str().ok_or_else(invalid)?;
    if allowed.is_some_and(|hosts| {
        !hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    }) {
        return Err(invalid());
    }
    url.set_fragment(None);
    Ok(url)
}

pub struct Http;

#[cfg(not(target_arch = "wasm32"))]
impl Transport for Http {
    fn send(&self, request: Request) -> Reply<'_> {
        Box::pin(async move {
            let network = || problem("simplefin_network", "Couldn't reach the SimpleFIN server");
            let host = request.url.host_str().ok_or_else(network)?;
            let mut builder = reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30));
            if request.url.domain().is_some() {
                let port = request.url.port_or_known_default().ok_or_else(network)?;
                let addresses: Vec<_> = tokio::net::lookup_host((host, port))
                    .await
                    .map_err(|_| network())?
                    .collect();
                if addresses.is_empty() || addresses.iter().any(|address| !public_ip(address.ip()))
                {
                    return Err(problem(
                        "simplefin_url",
                        "The SimpleFIN server must resolve to a public address",
                    ));
                }
                builder = builder.resolve_to_addrs(host, &addresses);
            }
            let client = builder.build().map_err(|_| network())?;
            let mut send = if request.claim {
                client
                    .post(request.url)
                    .header(reqwest::header::CONTENT_LENGTH, "0")
            } else {
                client.get(request.url)
            };
            if let Some(header) = request.authorization {
                send = send.header(reqwest::header::AUTHORIZATION, header);
            }
            let mut response = send.send().await.map_err(|_| network())?;
            let status = response.status().as_u16();
            let mut body = Vec::new();
            while let Some(bytes) = response.chunk().await.map_err(|_| network())? {
                if body.len().saturating_add(bytes.len()) > RESPONSE_LIMIT {
                    return Err(problem(
                        "import_too_large",
                        "The SimpleFIN response exceeds the import limit",
                    ));
                }
                body.extend_from_slice(&bytes);
            }
            Ok(Response { status, body })
        })
    }
}

#[cfg(target_arch = "wasm32")]
impl Transport for Http {
    fn send(&self, request: Request) -> Reply<'_> {
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::{RequestCredentials, RequestInit, RequestMode, RequestRedirect};
        Box::pin(async move {
            let network = || {
                problem("simplefin_network", "Couldn't reach the SimpleFIN server from this browser. Try the desktop app or CLI.")
            };
            // Fetch does not expose the DNS result or let the app pin it.
            if !request
                .url
                .host_str()
                .is_some_and(|host| BRIDGE_HOSTS.contains(&host))
            {
                return Err(problem(
                    "simplefin_url",
                    "Use the desktop app or CLI to connect to this SimpleFIN server",
                ));
            }
            let options = RequestInit::new();
            options.set_method(if request.claim { "POST" } else { "GET" });
            options.set_mode(RequestMode::Cors);
            options.set_credentials(RequestCredentials::Omit);
            options.set_redirect(RequestRedirect::Error);
            let outbound = web_sys::Request::new_with_str_and_init(request.url.as_str(), &options)
                .map_err(|_| network())?;
            if let Some(header) = request.authorization {
                outbound
                    .headers()
                    .set("Authorization", &header)
                    .map_err(|_| network())?;
            }
            let window = web_sys::window().ok_or_else(network)?;
            let response: web_sys::Response = JsFuture::from(window.fetch_with_request(&outbound))
                .await
                .map_err(|_| network())?
                .dyn_into()
                .map_err(|_| network())?;
            let bytes = JsFuture::from(response.array_buffer().map_err(|_| network())?)
                .await
                .map_err(|_| network())?;
            let bytes = js_sys::Uint8Array::new(&bytes);
            if bytes.length() as usize > RESPONSE_LIMIT {
                return Err(problem(
                    "import_too_large",
                    "The SimpleFIN response exceeds the import limit",
                ));
            }
            Ok(Response {
                status: response.status(),
                body: bytes.to_vec(),
            })
        })
    }
}
