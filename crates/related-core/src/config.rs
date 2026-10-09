//! User configuration and on-disk locations.
//!
//! The config lives at `<config dir>/spotify-related/config.toml`
//! (`~/.config` on Linux, `~/Library/Application Support` on macOS,
//! `%APPDATA%` on Windows). Environment variables override the file so the app
//! can be tried without writing one.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const APP_DIR: &str = "spotify-related";
pub const DEFAULT_REDIRECT_PORT: u16 = 8898;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Client ID of your app at developer.spotify.com. Required.
    pub spotify_client_id: String,
    /// Loopback port for the OAuth redirect. Register
    /// `http://127.0.0.1:<port>/callback` as a redirect URI on your app.
    pub redirect_port: u16,
    /// Optional Last.fm API key. Last.fm's track similarity is the strongest
    /// source, so it is worth the minute it takes to get a key.
    pub lastfm_api_key: Option<String>,
    pub sources: SourceConfig,
    /// At most this many related tracks by the same artist. Keeps the list from
    /// collapsing into the current artist's discography.
    pub max_per_artist: usize,
    /// How many related tracks to show.
    pub list_size: usize,
    /// Play audio in the app itself, as a Spotify Connect device.
    pub local_playback: LocalPlaybackConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalPlaybackConfig {
    pub enabled: bool,
    /// The name this computer shows in Spotify's device list.
    pub device_name: String,
    /// 96, 160 or 320 kbps.
    pub bitrate: u16,
    /// Even out loudness between tracks.
    pub normalise: bool,
    /// Keep recently played audio on disk (up to 1 GB) to save bandwidth.
    pub audio_cache: bool,
}

impl Default for LocalPlaybackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            device_name: "spotify-related".into(),
            bitrate: 320,
            normalise: false,
            audio_cache: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceConfig {
    pub lastfm_weight: f64,
    pub listenbrainz_weight: f64,
    pub deezer_weight: f64,
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self { lastfm_weight: 1.0, listenbrainz_weight: 1.0, deezer_weight: 0.6 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            spotify_client_id: String::new(),
            redirect_port: DEFAULT_REDIRECT_PORT,
            lastfm_api_key: None,
            sources: SourceConfig::default(),
            max_per_artist: 2,
            list_size: 40,
            local_playback: LocalPlaybackConfig::default(),
        }
    }
}

impl Config {
    /// Load the config file (if any), then apply `SPOTIFY_CLIENT_ID`,
    /// `LASTFM_API_KEY` and `SPOTIFY_REDIRECT_PORT` from the environment.
    pub fn load() -> Result<Self> {
        let mut config = match config_path() {
            Some(path) if path.exists() => {
                let text = fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
            }
            _ => Self::default(),
        };
        if let Ok(id) = std::env::var("SPOTIFY_CLIENT_ID") {
            config.spotify_client_id = id;
        }
        if let Ok(key) = std::env::var("LASTFM_API_KEY") {
            config.lastfm_api_key = Some(key);
        }
        if let Some(port) = std::env::var("SPOTIFY_REDIRECT_PORT").ok().and_then(|p| p.parse().ok())
        {
            config.redirect_port = port;
        }
        config.lastfm_api_key = config.lastfm_api_key.filter(|k| !k.trim().is_empty());
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path().context("no config directory on this platform")?;
        write_private(&path, toml::to_string_pretty(self)?.as_bytes())
    }

    pub fn is_ready(&self) -> bool {
        !self.spotify_client_id.trim().is_empty()
    }

    pub fn redirect_uri(&self) -> String {
        redirect_uri(self.redirect_port)
    }
}

/// Spotify only accepts loopback redirects as IP literals, not `localhost`.
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join("config.toml"))
}

pub fn token_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join("token.json"))
}

/// Reusable credentials and saved volume for the built-in player.
pub fn librespot_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join("librespot"))
}

pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join(APP_DIR))
}

/// Write a file readable only by the current user (on Unix). Used for the
/// config and the refresh token.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_keeps_defaults() {
        let config: Config = toml::from_str("spotify_client_id = \"abc\"\n").unwrap();
        assert_eq!(config.spotify_client_id, "abc");
        assert_eq!(config.redirect_port, DEFAULT_REDIRECT_PORT);
        assert_eq!(config.sources.deezer_weight, 0.6);
        assert!(config.local_playback.enabled);
        assert_eq!(config.local_playback.bitrate, 320);
        assert!(config.is_ready());
    }

    #[test]
    fn redirect_uses_ip_literal() {
        assert_eq!(redirect_uri(8898), "http://127.0.0.1:8898/callback");
    }
}
