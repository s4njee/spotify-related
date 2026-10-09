//! The built-in Spotify Connect device, when the app is built with the
//! `local-playback` feature. Without it every call reports that the device is
//! unavailable, so the rest of the app needs no feature checks.

use related_core::Config;

/// librespot's own OAuth redirect. The fallback sign-in uses librespot's
/// client ID, which accepts this loopback address.
pub const LIBRESPOT_REDIRECT_PORT: u16 = 5588;
pub const LIBRESPOT_REDIRECT: &str = "http://127.0.0.1:5588/login";

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "local-playback"), allow(dead_code))]
pub enum LocalState {
    Off,
    Starting,
    NeedsAuthorization(Option<String>),
    Connecting,
    Ready,
    Reconnecting,
    Failed(String),
}

impl LocalState {
    pub fn label(&self) -> String {
        match self {
            Self::Off => "Off".into(),
            Self::Starting | Self::Connecting => "Connecting".into(),
            Self::NeedsAuthorization(_) => "Needs permission".into(),
            Self::Ready => "Ready".into(),
            Self::Reconnecting => "Reconnecting".into(),
            Self::Failed(e) => format!("Unavailable: {e}"),
        }
    }
}

#[cfg(feature = "local-playback")]
pub struct LocalDevice(local_player::LocalPlayer);

#[cfg(feature = "local-playback")]
impl LocalDevice {
    pub fn start(config: &Config, access_token: Option<String>) -> Result<Self, String> {
        let settings = &config.local_playback;
        let credentials_dir =
            related_core::config::librespot_dir().ok_or("no config directory on this platform")?;
        let audio_cache_dir = settings
            .audio_cache
            .then(|| related_core::config::cache_dir().map(|d| d.join("audio")))
            .flatten();
        local_player::LocalPlayer::start(
            local_player::LocalPlayerConfig {
                device_name: settings.device_name.clone(),
                credentials_dir,
                audio_cache_dir,
                bitrate: settings.bitrate,
                initial_volume: 60,
                normalise: settings.normalise,
            },
            access_token,
        )
        .map(Self)
        .map_err(|e| e.to_string())
    }

    pub fn state(&self) -> LocalState {
        use local_player::Status;
        match self.0.status() {
            Status::Starting => LocalState::Starting,
            Status::NeedsAuthorization { reason } => LocalState::NeedsAuthorization(reason),
            Status::Connecting => LocalState::Connecting,
            Status::Ready { .. } => LocalState::Ready,
            Status::Reconnecting => LocalState::Reconnecting,
            Status::Failed(e) => LocalState::Failed(e),
            Status::Stopped => LocalState::Off,
        }
    }

    pub fn name(&self) -> &str {
        self.0.device_name()
    }

    pub fn authorize(&self, access_token: String) {
        self.0.authorize(access_token);
    }

    pub fn has_cached_credentials() -> bool {
        related_core::config::librespot_dir()
            .is_some_and(|d| local_player::LocalPlayer::has_cached_credentials(&d))
    }

    pub fn forget_credentials() {
        if let Some(dir) = related_core::config::librespot_dir() {
            local_player::LocalPlayer::forget_credentials(&dir);
        }
    }

    pub fn librespot_client_id() -> Option<String> {
        Some(local_player::LocalPlayer::librespot_client_id())
    }
}

#[cfg(not(feature = "local-playback"))]
pub struct LocalDevice;

#[cfg(not(feature = "local-playback"))]
impl LocalDevice {
    pub fn start(_: &Config, _: Option<String>) -> Result<Self, String> {
        Err("this build has no built-in player".into())
    }
    pub fn state(&self) -> LocalState {
        LocalState::Off
    }
    pub fn name(&self) -> &str {
        ""
    }
    pub fn authorize(&self, _: String) {}
    pub fn has_cached_credentials() -> bool {
        false
    }
    pub fn forget_credentials() {}
    pub fn librespot_client_id() -> Option<String> {
        None
    }
}
