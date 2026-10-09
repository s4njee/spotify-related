//! Play Spotify audio inside the app.
//!
//! This runs a [librespot](https://github.com/librespot-org/librespot)
//! Spotify Connect device on its own thread and tokio runtime. The device
//! shows up in Spotify's device list like any speaker, so the app keeps
//! controlling playback through the Web API exactly as it does for remote
//! devices; it simply has a device of its own to send audio to.
//!
//! Signing the device in needs an access token with the `streaming` scope.
//! After the first successful connection librespot stores reusable
//! credentials in the cache directory, so later starts need no token.

use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use librespot_connect::{ConnectConfig, Spirc};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::{DeviceType, SessionConfig};
use librespot_core::error::ErrorKind;
use librespot_core::session::Session;
use librespot_playback::audio_backend::{self, Sink, SinkError, SinkResult};
use librespot_playback::config::{AudioFormat, Bitrate, PlayerConfig};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::{self, MixerConfig};
use librespot_playback::player::{Player, PlayerEvent};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

/// Give up reconnecting after this many drops inside the window.
const RECONNECT_LIMIT: usize = 5;
const RECONNECT_WINDOW: Duration = Duration::from_secs(600);
const RETRY_DELAY: Duration = Duration::from_secs(5);
/// Upper bound for librespot's audio file cache.
const AUDIO_CACHE_BYTES: u64 = 1 << 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPlayerConfig {
    /// The name shown in Spotify's device list.
    pub device_name: String,
    /// Where librespot keeps reusable credentials and the saved volume.
    pub credentials_dir: PathBuf,
    /// Where librespot caches audio files. `None` disables the audio cache.
    pub audio_cache_dir: Option<PathBuf>,
    /// 96, 160 or 320 kbps.
    pub bitrate: u16,
    /// Starting volume, 0–100, used until librespot has saved one.
    pub initial_volume: u8,
    /// ReplayGain-style loudness normalisation.
    pub normalise: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    /// The device has no usable credentials. Call [`LocalPlayer::authorize`]
    /// with an access token that has the `streaming` scope.
    NeedsAuthorization {
        reason: Option<String>,
    },
    Connecting,
    Ready {
        username: String,
    },
    Reconnecting,
    Failed(String),
    Stopped,
}

impl Status {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

enum Command {
    Authorize(String),
    Shutdown,
}

/// A running Connect device. Dropping it disconnects the device.
pub struct LocalPlayer {
    device_name: String,
    status: Arc<Mutex<Status>>,
    commands: mpsc::UnboundedSender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl LocalPlayer {
    /// Start the device. `access_token` is used only when no credentials are
    /// cached yet; it must carry the `streaming` scope.
    pub fn start(config: LocalPlayerConfig, access_token: Option<String>) -> std::io::Result<Self> {
        let status = Arc::new(Mutex::new(Status::Starting));
        let (tx, rx) = mpsc::unbounded_channel();
        let device_name = config.device_name.clone();
        let thread_status = status.clone();
        let thread = std::thread::Builder::new().name("local-player".into()).spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("librespot")
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    *thread_status.lock().unwrap() = Status::Failed(format!("tokio: {e}"));
                    return;
                }
            };
            runtime.block_on(run(config, access_token, thread_status, rx));
        })?;
        Ok(Self { device_name, status, commands: tx, thread: Some(thread) })
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Supply an access token with the `streaming` scope after the device
    /// reported [`Status::NeedsAuthorization`].
    pub fn authorize(&self, access_token: String) {
        let _ = self.commands.send(Command::Authorize(access_token));
    }

    /// Whether a previous run left reusable credentials in `credentials_dir`.
    pub fn has_cached_credentials(credentials_dir: &Path) -> bool {
        Cache::new(Some(credentials_dir), None, None, None)
            .ok()
            .and_then(|c| c.credentials())
            .is_some()
    }

    /// Remove cached credentials, for signing out.
    pub fn forget_credentials(credentials_dir: &Path) {
        let _ = std::fs::remove_file(credentials_dir.join("credentials.json"));
    }

    /// The client ID librespot authenticates as. An access token from this
    /// client always has the privileges a Connect device needs, which makes
    /// it the fallback when a token from the user's own app is refused.
    pub fn librespot_client_id() -> String {
        SessionConfig::default().client_id
    }
}

impl Drop for LocalPlayer {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn set(status: &Mutex<Status>, value: Status) {
    log::info!("local player: {value:?}");
    *status.lock().unwrap() = value;
}

/// Stable per device name, so Spotify recognises the device across restarts.
fn device_id(name: &str) -> String {
    let digest = Sha256::digest(format!("spotify-related:{name}").as_bytes());
    digest.iter().take(20).map(|b| format!("{b:02x}")).collect()
}

/// Honour the usual proxy variables, as the librespot CLI does with `http_proxy`.
fn proxy_from_env() -> Option<url::Url> {
    ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .filter(|v| !v.is_empty())
        .and_then(|v| url::Url::parse(&v).ok())
}

fn bitrate(kbps: u16) -> Bitrate {
    match kbps {
        0..=128 => Bitrate::Bitrate96,
        129..=240 => Bitrate::Bitrate160,
        _ => Bitrate::Bitrate320,
    }
}

async fn run(
    config: LocalPlayerConfig,
    access_token: Option<String>,
    status: Arc<Mutex<Status>>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    let cache = match Cache::new(
        Some(config.credentials_dir.as_path()),
        Some(config.credentials_dir.as_path()),
        config.audio_cache_dir.as_deref(),
        config.audio_cache_dir.as_ref().map(|_| AUDIO_CACHE_BYTES),
    ) {
        Ok(cache) => cache,
        Err(e) => return set(&status, Status::Failed(format!("cache: {e}"))),
    };
    let session_config = SessionConfig {
        device_id: device_id(&config.device_name),
        proxy: proxy_from_env(),
        ..SessionConfig::default()
    };
    let initial_volume = cache
        .volume()
        .unwrap_or((config.initial_volume.min(100) as u32 * u16::MAX as u32 / 100) as u16);
    let connect_config = ConnectConfig {
        name: config.device_name.clone(),
        device_type: DeviceType::Computer,
        initial_volume,
        ..ConnectConfig::default()
    };
    let player_config = PlayerConfig {
        bitrate: bitrate(config.bitrate),
        normalisation: config.normalise,
        ..PlayerConfig::default()
    };

    let Some(mixer_fn) = mixer::find(Some("softvol")).or_else(|| mixer::find(None)) else {
        return set(&status, Status::Failed("no software mixer available".into()));
    };
    let mixer = match mixer_fn(MixerConfig::default()) {
        Ok(m) => m,
        Err(e) => return set(&status, Status::Failed(format!("mixer: {e}"))),
    };
    // librespot's rodio backend panics on its player thread when there is no
    // output device, so check first and say so plainly.
    if !has_output_device() {
        return set(&status, Status::Failed("no audio output device found".into()));
    }
    let Some(sink_builder) =
        audio_backend::find(Some("rodio".into())).or_else(|| audio_backend::find(None))
    else {
        return set(&status, Status::Failed("no audio output backend available".into()));
    };

    let mut session = Session::new(session_config.clone(), Some(cache.clone()));
    let audio_status = status.clone();
    let player = Player::new(player_config, session.clone(), mixer.get_soft_volume(), move || {
        // Some backends panic when the device cannot be opened. Contain that,
        // report it, and keep the Connect device alive with a silent sink.
        match std::panic::catch_unwind(|| sink_builder(None, AudioFormat::default())) {
            Ok(sink) => sink,
            Err(_) => {
                set(&audio_status, Status::Failed("could not open the audio output device".into()));
                Box::new(NoAudio)
            }
        }
    });
    // Give the player thread a moment to open the device, so a failure shows
    // before we register with Spotify.
    tokio::time::sleep(Duration::from_millis(300)).await;
    if matches!(*status.lock().unwrap(), Status::Failed(_)) {
        return;
    }
    tokio::spawn(log_player_events(player.get_player_event_channel()));

    let mut credentials = cache
        .credentials()
        .map(|c| (c, true))
        .or_else(|| access_token.map(|t| (Credentials::with_access_token(t), false)));
    let mut drops: Vec<Instant> = Vec::new();
    let mut retry_delay = RETRY_DELAY;

    'outer: loop {
        let Some((creds, cached)) = credentials.clone() else {
            if !matches!(*status.lock().unwrap(), Status::NeedsAuthorization { .. }) {
                set(&status, Status::NeedsAuthorization { reason: None });
            }
            match commands.recv().await {
                Some(Command::Authorize(token)) => {
                    credentials = Some((Credentials::with_access_token(token), false));
                    continue;
                }
                Some(Command::Shutdown) | None => break,
            }
        };

        if session.is_invalid() {
            session = Session::new(session_config.clone(), Some(cache.clone()));
            player.set_session(session.clone());
        }
        set(&status, Status::Connecting);

        let connect = Spirc::new(
            connect_config.clone(),
            session.clone(),
            creds,
            player.clone(),
            mixer.clone(),
        );
        let started = tokio::select! {
            result = connect => result,
            command = commands.recv() => match command {
                Some(Command::Authorize(token)) => {
                    credentials = Some((Credentials::with_access_token(token), false));
                    continue;
                }
                Some(Command::Shutdown) | None => break,
            },
        };

        let (spirc, task) = match started {
            Ok(pair) => pair,
            Err(e) => {
                log::warn!("local player could not connect: {e}");
                let message = e.to_string();
                if message.contains("Premium account required") {
                    set(
                        &status,
                        Status::Failed("Spotify Premium is required to play audio".into()),
                    );
                    credentials = None;
                    if wait_or_command(&mut commands, &mut credentials, Duration::from_secs(86_400))
                        .await
                    {
                        break;
                    }
                } else if matches!(e.kind, ErrorKind::Unauthenticated | ErrorKind::PermissionDenied)
                {
                    if cached {
                        LocalPlayer::forget_credentials(&config.credentials_dir);
                    }
                    credentials = None;
                    set(&status, Status::NeedsAuthorization { reason: Some(message) });
                } else {
                    // Network trouble: keep the credentials and retry with backoff.
                    set(&status, Status::Failed(format!("can't reach Spotify ({message})")));
                    if wait_or_command(&mut commands, &mut credentials, retry_delay).await {
                        break;
                    }
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(60));
                }
                continue;
            }
        };

        // Spirc stored reusable credentials; prefer them from now on.
        credentials = cache.credentials().map(|c| (c, true)).or(credentials);
        retry_delay = RETRY_DELAY;
        if player.is_invalid() {
            let _ = spirc.shutdown();
            return set(&status, Status::Failed("audio output failed to start".into()));
        }
        set(&status, Status::Ready { username: session.username() });

        let mut task = pin!(task);
        loop {
            tokio::select! {
                _ = &mut task => break,
                command = commands.recv() => match command {
                    // Already signed in; a late token changes nothing.
                    Some(Command::Authorize(_)) => continue,
                    Some(Command::Shutdown) | None => {
                        let _ = spirc.shutdown();
                        let _ = tokio::time::timeout(Duration::from_secs(3), &mut task).await;
                        break 'outer;
                    }
                },
            }
        }

        log::warn!("local player disconnected from Spotify");
        drops.retain(|t| t.elapsed() < RECONNECT_WINDOW);
        drops.push(Instant::now());
        if drops.len() > RECONNECT_LIMIT {
            set(&status, Status::Failed("Spotify keeps disconnecting this device".into()));
            if wait_or_command(&mut commands, &mut credentials, Duration::from_secs(300)).await {
                break;
            }
            drops.clear();
        } else {
            set(&status, Status::Reconnecting);
            if !session.is_invalid() {
                session.shutdown();
            }
            if wait_or_command(&mut commands, &mut credentials, Duration::from_secs(2)).await {
                break;
            }
        }
    }
    set(&status, Status::Stopped);
}

/// Stands in for an audio device that failed to open.
struct NoAudio;

impl Sink for NoAudio {
    fn write(&mut self, _: AudioPacket, _: &mut Converter) -> SinkResult<()> {
        Err(SinkError::NotConnected("no audio output device".into()))
    }
}

fn has_output_device() -> bool {
    use cpal::traits::HostTrait;
    cpal::default_host().default_output_device().is_some()
}

/// Sleep, but wake early for a command. Returns true when asked to shut down.
async fn wait_or_command(
    commands: &mut mpsc::UnboundedReceiver<Command>,
    credentials: &mut Option<(Credentials, bool)>,
    delay: Duration,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        command = commands.recv() => match command {
            Some(Command::Authorize(token)) => {
                *credentials = Some((Credentials::with_access_token(token), false));
                false
            }
            Some(Command::Shutdown) | None => true,
        },
    }
}

async fn log_player_events(mut events: mpsc::UnboundedReceiver<PlayerEvent>) {
    while let Some(event) = events.recv().await {
        match event {
            PlayerEvent::Unavailable { track_id, .. } => {
                log::warn!("local player: track {track_id:?} is unavailable");
            }
            PlayerEvent::SessionDisconnected { .. } => {
                log::info!("local player: session disconnected")
            }
            other => log::debug!("local player: {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_stable_and_hex() {
        let a = device_id("spotify-related");
        assert_eq!(a, device_id("spotify-related"));
        assert_ne!(a, device_id("Kitchen"));
        assert_eq!(a.len(), 40);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn bitrate_rounds_to_supported_values() {
        assert_eq!(bitrate(96), Bitrate::Bitrate96);
        assert_eq!(bitrate(160), Bitrate::Bitrate160);
        assert_eq!(bitrate(320), Bitrate::Bitrate320);
    }

    #[test]
    fn no_cached_credentials_in_empty_dir() {
        let dir = std::env::temp_dir().join(format!("local-player-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!LocalPlayer::has_cached_credentials(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
