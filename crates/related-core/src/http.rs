//! Shared HTTP agent.

use std::time::Duration;

pub use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

/// Identifies the app to the open-data services. MusicBrainz asks every client
/// for a meaningful user agent with a way to reach the maintainer.
pub const USER_AGENT: &str = concat!(
    "spotify-related/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/s4njee/spotify-related )"
);

/// Build the agent every client in this crate uses.
///
/// Status codes are returned instead of raised so each client can map its own
/// API errors. The platform verifier trusts the OS certificate store, which
/// keeps the app working behind corporate TLS proxies. Proxy settings come
/// from the standard `HTTPS_PROXY` / `NO_PROXY` variables.
pub fn agent() -> Agent {
    Agent::config_builder()
        .user_agent(USER_AGENT)
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
}

/// Download a small binary resource such as album art. `None` on any failure.
pub fn fetch_bytes(agent: &Agent, url: &str, max_bytes: u64) -> Option<Vec<u8>> {
    let mut response = agent.get(url).call().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.body_mut().with_config().limit(max_bytes).read_to_vec().ok()
}
