//! Spotify Web API client for playback control and track search.
//!
//! Playback goes through Spotify Connect: the app controls a device that is
//! already running Spotify (the desktop or phone app, a speaker, or a
//! librespot-based daemon such as spotifyd). Controlling playback needs
//! Spotify Premium.

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Value, json};
use ureq::Agent;

use super::auth::{self, Token};
use crate::config;
use crate::model::{Device, PlaybackState, RepeatMode, Track};

const API: &str = "https://api.spotify.com/v1";

#[derive(Debug, thiserror::Error)]
pub enum SpotifyError {
    #[error("Not signed in to Spotify")]
    SignedOut,
    #[error("No Spotify device is available. Open Spotify on this computer or your phone.")]
    NoDevice,
    #[error("Spotify Premium is required to control playback")]
    PremiumRequired,
    #[error("Spotify is rate limiting requests; retry in {0}s")]
    RateLimited(u64),
    #[error("Spotify returned {status}: {message}")]
    Api { status: u16, message: String },
    #[error("{0}")]
    Other(String),
}

impl From<ureq::Error> for SpotifyError {
    fn from(e: ureq::Error) -> Self {
        Self::Other(format!("network error: {e}"))
    }
}

impl From<anyhow::Error> for SpotifyError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(format!("{e:#}"))
    }
}

pub type Result<T> = std::result::Result<T, SpotifyError>;

#[derive(Clone, Copy)]
enum Method {
    Get,
    Put,
    Post,
}

pub struct SpotifyClient {
    agent: Agent,
    client_id: String,
    token: Mutex<Option<Token>>,
    token_path: Option<PathBuf>,
    /// Device to use when nothing is active, such as the built-in player.
    preferred_device: Mutex<Option<String>>,
}

impl SpotifyClient {
    /// Create a client and load a saved token, if there is one.
    pub fn new(client_id: &str) -> Self {
        let token_path = config::token_path();
        let token = token_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<Token>(&s).ok());
        Self {
            agent: crate::http::agent(),
            client_id: client_id.to_owned(),
            token: Mutex::new(token),
            token_path,
            preferred_device: Mutex::new(None),
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    pub fn is_signed_in(&self) -> bool {
        self.token.lock().unwrap().is_some()
    }

    /// Whether the saved sign-in granted `scope`.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.token.lock().unwrap().as_ref().is_some_and(|t| t.has_scope(scope))
    }

    /// A valid access token, refreshed if it is about to expire.
    pub fn fresh_access_token(&self) -> Result<String> {
        self.access_token(false)
    }

    /// Prefer the device with this name when playback has to pick one.
    pub fn set_preferred_device(&self, name: Option<String>) {
        *self.preferred_device.lock().unwrap() = name;
    }

    pub fn set_token(&self, token: Token) {
        self.persist(&token);
        *self.token.lock().unwrap() = Some(token);
    }

    pub fn sign_out(&self) {
        *self.token.lock().unwrap() = None;
        if let Some(path) = &self.token_path {
            let _ = std::fs::remove_file(path);
        }
    }

    fn persist(&self, token: &Token) {
        if let Some(path) = &self.token_path {
            let bytes = serde_json::to_vec(token).expect("token serializes");
            if let Err(e) = config::write_private(path, &bytes) {
                log::warn!("could not save Spotify token: {e:#}");
            }
        }
    }

    fn access_token(&self, force_refresh: bool) -> Result<String> {
        let mut guard = self.token.lock().unwrap();
        let token = guard.as_ref().ok_or(SpotifyError::SignedOut)?;
        if force_refresh || token.expires_within(60) {
            match auth::refresh(&self.agent, &self.client_id, token) {
                Ok(fresh) => {
                    self.persist(&fresh);
                    *guard = Some(fresh);
                }
                Err(e) => {
                    log::warn!("token refresh failed: {e:#}");
                    *guard = None;
                    return Err(SpotifyError::SignedOut);
                }
            }
        }
        Ok(guard.as_ref().expect("token present").access_token.clone())
    }

    /// Send a request and return the JSON body, or `None` for an empty body.
    /// Refreshes the token once on 401.
    fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Option<Value>> {
        let mut retried = false;
        loop {
            let token = self.access_token(retried)?;
            let url = format!("{API}{path}");
            let auth = format!("Bearer {token}");
            let response = match method {
                Method::Get => {
                    let mut req = self.agent.get(&url).header("Authorization", &auth);
                    for (k, v) in query {
                        req = req.query(*k, v);
                    }
                    req.call()?
                }
                Method::Put | Method::Post => {
                    let mut req = if matches!(method, Method::Put) {
                        self.agent.put(&url)
                    } else {
                        self.agent.post(&url)
                    };
                    req = req.header("Authorization", &auth);
                    for (k, v) in query {
                        req = req.query(*k, v);
                    }
                    match &body {
                        Some(json) => req.send_json(json)?,
                        None => req.send_empty()?,
                    }
                }
            };
            let status = response.status().as_u16();
            if status == 401 && !retried {
                retried = true;
                continue;
            }
            return read_response(response, status);
        }
    }

    // ----- player -------------------------------------------------------

    /// Current playback, or `None` when nothing is playing anywhere.
    pub fn playback(&self) -> Result<Option<PlaybackState>> {
        let v =
            self.call(Method::Get, "/me/player", &[("additional_types", "track".into())], None)?;
        Ok(v.map(|v| PlaybackState::from_json(&v)))
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        let v = self.call(Method::Get, "/me/player/devices", &[], None)?.unwrap_or_default();
        Ok(v.get("devices")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Device::from_json).collect())
            .unwrap_or_default())
    }

    /// Play these tracks in order, replacing the current context. Falls back to
    /// the first available device when none is active.
    pub fn play_tracks(&self, uris: &[String]) -> Result<()> {
        self.with_device_fallback(|device| {
            let mut query = Vec::new();
            if let Some(id) = device {
                query.push(("device_id", id.to_owned()));
            }
            self.call(Method::Put, "/me/player/play", &query, Some(json!({ "uris": uris })))
                .map(drop)
        })
    }

    pub fn resume(&self) -> Result<()> {
        self.with_device_fallback(|device| {
            let query: Vec<_> = device.map(|id| ("device_id", id.to_owned())).into_iter().collect();
            self.call(Method::Put, "/me/player/play", &query, None).map(drop)
        })
    }

    pub fn pause(&self) -> Result<()> {
        self.call(Method::Put, "/me/player/pause", &[], None).map(drop)
    }

    pub fn next(&self) -> Result<()> {
        self.call(Method::Post, "/me/player/next", &[], None).map(drop)
    }

    pub fn previous(&self) -> Result<()> {
        self.call(Method::Post, "/me/player/previous", &[], None).map(drop)
    }

    pub fn seek(&self, position_ms: u64) -> Result<()> {
        self.call(Method::Put, "/me/player/seek", &[("position_ms", position_ms.to_string())], None)
            .map(drop)
    }

    pub fn set_volume(&self, percent: u8) -> Result<()> {
        let query = [("volume_percent", percent.min(100).to_string())];
        self.call(Method::Put, "/me/player/volume", &query, None).map(drop)
    }

    pub fn set_shuffle(&self, on: bool) -> Result<()> {
        self.call(Method::Put, "/me/player/shuffle", &[("state", on.to_string())], None).map(drop)
    }

    pub fn set_repeat(&self, mode: RepeatMode) -> Result<()> {
        self.call(Method::Put, "/me/player/repeat", &[("state", mode.as_api().into())], None)
            .map(drop)
    }

    pub fn add_to_queue(&self, uri: &str) -> Result<()> {
        self.with_device_fallback(|device| {
            let mut query = vec![("uri", uri.to_owned())];
            if let Some(id) = device {
                query.push(("device_id", id.to_owned()));
            }
            self.call(Method::Post, "/me/player/queue", &query, None).map(drop)
        })
    }

    pub fn transfer(&self, device_id: &str, play: bool) -> Result<()> {
        let body = json!({ "device_ids": [device_id], "play": play });
        self.call(Method::Put, "/me/player", &[], Some(body)).map(drop)
    }

    fn with_device_fallback(&self, f: impl Fn(Option<&str>) -> Result<()>) -> Result<()> {
        match f(None) {
            Err(SpotifyError::NoDevice) => {
                let devices = self.devices()?;
                let device = devices
                    .iter()
                    .find(|d| d.is_active)
                    .or_else(|| {
                        let preferred = self.preferred_device.lock().unwrap().clone()?;
                        devices.iter().find(|d| d.name == preferred)
                    })
                    .or_else(|| devices.iter().find(|d| d.kind == "Computer"))
                    .or_else(|| devices.first())
                    .and_then(|d| d.id.clone())
                    .ok_or(SpotifyError::NoDevice)?;
                f(Some(&device))
            }
            other => other,
        }
    }

    // ----- catalog ------------------------------------------------------

    pub fn search_tracks(&self, query: &str, limit: usize) -> Result<Vec<Track>> {
        let q = [
            ("q", query.to_owned()),
            ("type", "track".into()),
            ("limit", limit.clamp(1, 10).to_string()),
        ];
        let v = self.call(Method::Get, "/search", &q, None)?.unwrap_or_default();
        Ok(v.pointer("/tracks/items")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Track::from_json).collect())
            .unwrap_or_default())
    }
}

fn read_response(
    mut response: ureq::http::Response<ureq::Body>,
    status: u16,
) -> Result<Option<Value>> {
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| SpotifyError::Other(format!("reading response: {e}")))?;
    let json: Option<Value> =
        if text.trim().is_empty() { None } else { serde_json::from_str(&text).ok() };
    if (200..300).contains(&status) {
        return Ok(json);
    }
    let message = json
        .as_ref()
        .and_then(|j| j.pointer("/error/message").and_then(Value::as_str))
        .unwrap_or(text.as_str())
        .to_owned();
    let reason = json.as_ref().and_then(|j| j.pointer("/error/reason")).and_then(Value::as_str);
    Err(match (status, reason) {
        (401, _) => SpotifyError::SignedOut,
        (429, _) => SpotifyError::RateLimited(retry_after),
        (404, Some("NO_ACTIVE_DEVICE")) => SpotifyError::NoDevice,
        (404, _) if message.to_lowercase().contains("no active device") => SpotifyError::NoDevice,
        (403, Some("PREMIUM_REQUIRED")) => SpotifyError::PremiumRequired,
        (403, _) if message.to_lowercase().contains("premium") => SpotifyError::PremiumRequired,
        _ => SpotifyError::Api { status, message },
    })
}
