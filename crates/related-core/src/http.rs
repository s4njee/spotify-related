//! Shared HTTP agent.

use std::time::Duration;

use ureq::Agent;
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
