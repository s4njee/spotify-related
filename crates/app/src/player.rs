//! The player window's state and everything it does off the UI thread.
//!
//! Rendering lives in `ui/`. This file owns the Spotify session, the playback
//! poll, the related-track pipeline (fetch → fuse → resolve), search, radio
//! mode and album-art loading.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui_pre::{
    App, AppContext, Context, Entity, FocusHandle, Subscription, Task, UniformListScrollHandle,
    Window,
};
use mkit::search_field::{SearchChanged, SearchField};
use mkit::segmented_control::{self, SegmentedControl};
use mkit::slider::{InteractionEnded, InteractionStarted, Slider};
use mkit::text_field::TextField;
use related_core::http::Agent;
use related_core::related::Resolution;
use related_core::spotify::SpotifyError;
use related_core::spotify::auth::{PendingAuth, PkceSession};
use related_core::{
    Config, Device, FusionOptions, PlaybackState, RelatedEngine, RelatedReport, RepeatMode, Seed,
    SpotifyClient, Track,
};

use crate::images::{self, ArtCache};
use crate::local::{LIBRESPOT_REDIRECT, LIBRESPOT_REDIRECT_PORT, LocalDevice, LocalState};

/// How often the UI redraws the progress bar while playing.
const TICK: Duration = Duration::from_millis(500);
/// Poll Spotify every this many ticks.
const POLL_EVERY: u32 = 3;
/// Refresh the device list every this many polls.
const DEVICES_EVERY: u32 = 10;
/// Radio mode queues the next track when this much of the current one is left.
const RADIO_LEAD_MS: u64 = 25_000;
/// Resolve related tracks on Spotify in batches of this size.
const RESOLVE_BATCH: usize = 8;
const HISTORY_LEN: usize = 24;

type PollResult =
    (Result<Option<PlaybackState>, SpotifyError>, Option<Result<Vec<Device>, SpotifyError>>);

pub struct Core {
    pub spotify: Arc<SpotifyClient>,
    pub engine: Arc<RelatedEngine>,
    pub agent: Agent,
}

impl Core {
    fn new(config: &Config) -> Self {
        let spotify = Arc::new(SpotifyClient::new(&config.spotify_client_id));
        let engine = Arc::new(RelatedEngine::new(config, spotify.clone()));
        Self { spotify, engine, agent: related_core::http::agent() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Error,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub kind: NoticeKind,
    pub message: String,
}

pub struct Player {
    /// `--demo`: no Spotify account; related tracks for a sample song.
    pub demo: bool,
    pub config: Config,
    pub core: Option<Core>,
    pub focus: FocusHandle,
    /// Keyboard focus for the related list (arrows, Enter, Q, E, Space).
    pub list_focus: FocusHandle,

    // Setup screen
    pub client_id_field: Entity<TextField>,
    pub lastfm_field: Entity<TextField>,
    pub signing_in: Option<Arc<AtomicBool>>,
    pub setup_error: Option<String>,

    // Playback
    pub playback: Option<PlaybackState>,
    polled_at: Instant,
    poll_now: bool,
    poll_task: Option<Task<()>>,
    pub devices: Vec<Device>,
    pub seek: Entity<Slider>,
    seek_for: Option<String>,
    seeking: bool,
    pub volume: Entity<Slider>,
    volume_dragging: bool,

    // Related tracks
    /// Follow the playing track, or stay on a seed the user picked.
    pub follow: bool,
    pub seed_mode: Entity<SegmentedControl>,
    pub seed: Option<Track>,
    pub report: Option<RelatedReport>,
    pub loading_related: bool,
    pub resolving: bool,
    related_generation: u64,
    pub selected: Option<usize>,
    pub list_scroll: UniformListScrollHandle,
    pub history: Vec<Track>,

    // Search
    pub search: Entity<SearchField>,
    pub search_query: String,
    pub search_results: Vec<Track>,
    pub searching: bool,
    search_generation: u64,

    // Radio mode
    pub radio: bool,
    radio_played: HashSet<String>,
    radio_queued_for: Option<String>,

    // Built-in player
    pub local: Option<LocalDevice>,
    pub local_state: LocalState,
    /// Whether this run already made the built-in player the active device.
    local_activated: bool,
    pub authorizing_local: Option<Arc<AtomicBool>>,
    devices_now: bool,

    pub art: ArtCache,
    pub notice: Option<Notice>,
    _subscriptions: Vec<Subscription>,
    seek_subscriptions: Vec<Subscription>,
}

impl Player {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = Config::load().unwrap_or_else(|e| {
            log::error!("config: {e:#}");
            Config::default()
        });
        let client_id = config.spotify_client_id.clone();
        let lastfm = config.lastfm_api_key.clone().unwrap_or_default();
        let client_id_field = cx.new(|cx| {
            TextField::new(cx)
                .with_label("Spotify client ID")
                .with_placeholder("32-character ID from your Spotify app")
                .controlled(client_id)
        });
        let lastfm_field = cx.new(|cx| {
            TextField::new(cx)
                .with_label("Last.fm API key (optional)")
                .with_placeholder("Adds Last.fm's similar-track data")
                .with_description("Get one free at last.fm/api/account/create.")
                .secure(true)
                .controlled(lastfm)
        });
        let search = cx.new(|_| {
            SearchField::new("Search Spotify")
                .placeholder("Search songs")
                .debounce(Duration::from_millis(350))
        });
        let volume = cx.new(|_| Slider::new("Volume", 50.0, 0.0, 100.0, 1.0));
        let seek = cx.new(|_| Slider::new("Seek", 0.0, 0.0, 1.0, 1.0).disabled(true));
        let seed_mode = cx.new(|_| {
            SegmentedControl::controlled(
                "Related to",
                vec![
                    segmented_control::Item::new("follow", "Now playing")
                        .tooltip("Follow whatever is playing"),
                    segmented_control::Item::new("pinned", "Pinned")
                        .tooltip("Keep this list while other songs play"),
                ],
                Some("follow".into()),
            )
        });

        let mut subscriptions = vec![
            cx.subscribe(&search, |this, _, event: &SearchChanged, cx| {
                this.run_search(event.query.clone(), cx);
            }),
            cx.subscribe(&seed_mode, |this, _, event: &segmented_control::ValueChanged, cx| {
                this.set_follow(event.0.as_deref() != Some("pinned"), cx);
            }),
            cx.subscribe(&volume, |this, _, _: &InteractionStarted, _| this.volume_dragging = true),
            cx.subscribe(&volume, |this, _, event: &InteractionEnded, cx| {
                this.volume_dragging = false;
                let percent = event.0.first().copied().unwrap_or(0.0).round() as u8;
                if let Some(device) = this.playback.as_mut().and_then(|p| p.device.as_mut()) {
                    device.volume_percent = Some(percent);
                }
                this.spotify_action(cx, move |s| s.set_volume(percent));
            }),
        ];
        subscriptions.push(cx.on_app_quit(|this, _| {
            // Disconnect the built-in device cleanly so it leaves Spotify's device list.
            let local = this.local.take();
            async move { drop(local) }
        }));
        subscriptions.push(cx.on_release(|this, _| {
            if let Some(cancel) = &this.signing_in {
                cancel.store(true, Ordering::Relaxed);
            }
            if let Some(core) = &this.core {
                core.engine.save_cache();
            }
        }));

        let focus = cx.focus_handle();
        let list_focus = cx.focus_handle();
        window.focus(&list_focus, cx);

        let mut player = Self {
            demo: false,
            core: None,
            focus,
            list_focus,
            client_id_field,
            lastfm_field,
            signing_in: None,
            setup_error: None,
            playback: None,
            polled_at: Instant::now(),
            poll_now: true,
            poll_task: None,
            devices: Vec::new(),
            seek,
            seek_for: None,
            seeking: false,
            volume,
            volume_dragging: false,
            follow: true,
            seed_mode,
            seed: None,
            report: None,
            loading_related: false,
            resolving: false,
            related_generation: 0,
            selected: None,
            list_scroll: UniformListScrollHandle::new(),
            history: Vec::new(),
            search,
            search_query: String::new(),
            search_results: Vec::new(),
            searching: false,
            search_generation: 0,
            radio: false,
            radio_played: HashSet::new(),
            radio_queued_for: None,
            local: None,
            local_state: LocalState::Off,
            local_activated: false,
            authorizing_local: None,
            devices_now: false,
            art: ArtCache::default(),
            notice: None,
            _subscriptions: subscriptions,
            seek_subscriptions: Vec::new(),
            config,
        };
        if std::env::args().any(|a| a == "--demo") {
            player.start_demo(cx);
        } else if player.config.is_ready() {
            let core = Core::new(&player.config);
            if core.spotify.is_signed_in() {
                player.core = Some(core);
                player.start_polling(cx);
                player.start_local(cx);
            }
        }
        player
    }

    /// Show the main window with a sample song and live related tracks, but
    /// without a Spotify account: nothing plays and nothing is resolved.
    fn start_demo(&mut self, cx: &mut Context<Self>) {
        self.demo = true;
        self.core = Some(Core::new(&self.config));
        let track = Track {
            id: "demo-seed".into(),
            uri: String::new(),
            name: "Blinding Lights".into(),
            artists: vec!["The Weeknd".into()],
            album: "After Hours".into(),
            album_art: None,
            duration_ms: 200_040,
            isrc: Some("USUG11904206".into()),
        };
        let device = Device {
            id: None,
            name: "Demo".into(),
            kind: "Computer".into(),
            is_active: true,
            volume_percent: Some(64),
        };
        self.devices = vec![device.clone()];
        self.playback = Some(PlaybackState {
            is_playing: false,
            progress_ms: 74_000,
            track: Some(track.clone()),
            large_art: None,
            device: Some(device),
            shuffle: false,
            repeat: RepeatMode::Off,
        });
        self.volume.update(cx, |s, cx| s.set_value(vec![64.0], cx));
        self.sync_seek(cx);
        self.notice = Some(Notice {
            kind: NoticeKind::Info,
            message: "Demo mode: related tracks are live, playback is off".into(),
        });
        self.set_seed(track, cx);
    }

    pub fn is_signed_in(&self) -> bool {
        self.demo || self.core.as_ref().is_some_and(|c| c.spotify.is_signed_in())
    }

    pub fn has_lastfm(&self) -> bool {
        self.config.lastfm_api_key.is_some()
    }

    // ----- sign-in ------------------------------------------------------

    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.signing_in.is_some() {
            return;
        }
        let client_id = self.client_id_field.read(cx).text().trim().to_owned();
        let lastfm = self.lastfm_field.read(cx).text().trim().to_owned();
        if client_id.len() < 16 || !client_id.chars().all(|c| c.is_ascii_alphanumeric()) {
            self.setup_error = Some("Enter the client ID shown on your Spotify app's page.".into());
            cx.notify();
            return;
        }
        self.config.spotify_client_id = client_id;
        self.config.lastfm_api_key = (!lastfm.is_empty()).then_some(lastfm);
        if let Err(e) = self.config.save() {
            log::warn!("saving config: {e:#}");
        }
        let pending =
            match PendingAuth::start(&self.config.spotify_client_id, self.config.redirect_port) {
                Ok(p) => p,
                Err(e) => {
                    self.setup_error = Some(format!("{e:#}"));
                    cx.notify();
                    return;
                }
            };
        cx.open_url(pending.url());
        let core = Core::new(&self.config);
        let spotify = core.spotify.clone();
        let agent = core.agent.clone();
        self.core = Some(core);
        let cancel = Arc::new(AtomicBool::new(false));
        self.signing_in = Some(cancel.clone());
        self.setup_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { pending.finish(&agent, cancel) }).await;
            this.update(cx, |this, cx| {
                this.signing_in = None;
                match result {
                    Ok(token) => {
                        spotify.set_token(token);
                        this.notice = None;
                        this.start_polling(cx);
                        this.start_local(cx);
                    }
                    Err(e) => this.setup_error = Some(format!("{e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.signing_in.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.demo = false;
        if let Some(core) = self.core.take() {
            core.spotify.sign_out();
            core.engine.save_cache();
        }
        self.poll_task = None;
        self.playback = None;
        self.report = None;
        self.seed = None;
        // Dropping the device disconnects it; forgetting its credentials makes
        // the next account sign it in afresh.
        self.local = None;
        self.local_state = LocalState::Off;
        self.local_activated = false;
        LocalDevice::forget_credentials();
        cx.notify();
    }

    // ----- built-in player ----------------------------------------------

    /// Start the built-in Connect device, signing it in with the app's own
    /// token the first time. librespot caches credentials after that.
    fn start_local(&mut self, cx: &mut Context<Self>) {
        if self.demo || self.local.is_some() || !self.config.local_playback.enabled {
            return;
        }
        let Some(core) = &self.core else { return };
        let spotify = core.spotify.clone();
        spotify.set_preferred_device(Some(self.config.local_playback.device_name.clone()));
        let use_app_token =
            !LocalDevice::has_cached_credentials() && spotify.has_scope("streaming");
        self.local_state = LocalState::Starting;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let token = if use_app_token {
                cx.background_spawn(async move { spotify.fresh_access_token().ok() }).await
            } else {
                None
            };
            this.update(cx, |this, cx| {
                if this.local.is_some() || this.core.is_none() {
                    return;
                }
                match LocalDevice::start(&this.config, token) {
                    Ok(device) => this.local = Some(device),
                    Err(e) => this.local_state = LocalState::Failed(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn restart_local(&mut self, cx: &mut Context<Self>) {
        self.local = None;
        self.local_activated = false;
        self.start_local(cx);
    }

    /// Fallback sign-in for the built-in player: authorize librespot's own
    /// client for the `streaming` scope, as the librespot CLI does.
    pub fn authorize_local(&mut self, cx: &mut Context<Self>) {
        if self.authorizing_local.is_some() {
            return;
        }
        let (Some(core), Some(client_id)) = (&self.core, LocalDevice::librespot_client_id()) else {
            return;
        };
        let session = PkceSession::with_scopes(&client_id, LIBRESPOT_REDIRECT, &["streaming"]);
        let pending = match PendingAuth::start_with(session, LIBRESPOT_REDIRECT_PORT) {
            Ok(p) => p,
            Err(e) => {
                self.notice = Some(Notice { kind: NoticeKind::Error, message: format!("{e:#}") });
                cx.notify();
                return;
            }
        };
        cx.open_url(pending.url());
        let agent = core.agent.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.authorizing_local = Some(cancel.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { pending.finish(&agent, cancel) }).await;
            this.update(cx, |this, cx| {
                this.authorizing_local = None;
                match result {
                    Ok(token) => match &this.local {
                        Some(local) => local.authorize(token.access_token),
                        None => {
                            if let Ok(device) =
                                LocalDevice::start(&this.config, Some(token.access_token))
                            {
                                this.local = Some(device);
                            }
                        }
                    },
                    Err(e) => {
                        this.notice =
                            Some(Notice { kind: NoticeKind::Error, message: format!("{e:#}") })
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn cancel_authorize_local(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.authorizing_local.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    fn tick_local(&mut self, cx: &mut Context<Self>) {
        let Some(local) = &self.local else { return };
        let state = local.state();
        if state != self.local_state {
            if state == LocalState::Ready {
                // Show the new device right away instead of at the next device refresh.
                self.poll_now = true;
                self.devices_now = true;
            }
            self.local_state = state;
            cx.notify();
        }
    }

    /// The first time the built-in device is online and nothing is playing
    /// anywhere, make it the active device so Play works straight away.
    fn activate_local_if_idle(&mut self, cx: &mut Context<Self>) {
        if self.local_activated || self.local_state != LocalState::Ready {
            return;
        }
        let Some(name) = self.local.as_ref().map(|l| l.name().to_owned()) else { return };
        let Some(id) = self.devices.iter().find(|d| d.name == name).and_then(|d| d.id.clone())
        else {
            return;
        };
        self.local_activated = true;
        if self.playback.as_ref().is_some_and(|p| p.device.is_some()) {
            return;
        }
        self.spotify_action(cx, move |s| s.transfer(&id, false));
    }

    pub fn is_local_device(&self, device: &Device) -> bool {
        self.local.as_ref().is_some_and(|l| l.name() == device.name)
    }

    // ----- playback poll ------------------------------------------------

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self.poll_now = true;
        self.poll_task = Some(cx.spawn(async move |this, cx| {
            let mut tick: u32 = 0;
            let mut polls: u32 = 0;
            loop {
                let due = this
                    .update(cx, |this, cx| {
                        this.tick_local(cx);
                        (std::mem::take(&mut this.poll_now), std::mem::take(&mut this.devices_now))
                    })
                    .ok();
                let Some((force, devices_now)) = due else { break };
                if force || tick.is_multiple_of(POLL_EVERY) {
                    let Ok(Some(spotify)) =
                        this.update(cx, |this, _| this.core.as_ref().map(|c| c.spotify.clone()))
                    else {
                        break;
                    };
                    let with_devices = devices_now || polls.is_multiple_of(DEVICES_EVERY);
                    let result = cx
                        .background_spawn(async move {
                            let playback = spotify.playback();
                            let devices = with_devices.then(|| spotify.devices());
                            (playback, devices)
                        })
                        .await;
                    polls += 1;
                    if this.update(cx, |this, cx| this.apply_poll(result, cx)).is_err() {
                        break;
                    }
                } else if this
                    .update(cx, |this, cx| {
                        if this.playback.as_ref().is_some_and(|p| p.is_playing) {
                            this.sync_seek(cx);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
                tick = tick.wrapping_add(1);
                cx.background_executor().timer(TICK).await;
            }
        }));
    }

    fn apply_poll(&mut self, (playback, devices): PollResult, cx: &mut Context<Self>) {
        if let Some(Ok(devices)) = devices {
            self.devices = devices;
        }
        let playback = match playback {
            Ok(p) => p,
            Err(e) => {
                self.handle_error(e, cx);
                return;
            }
        };
        let previous = self.playback.as_ref().and_then(|p| p.track.as_ref()).map(|t| t.id.clone());
        self.playback = playback;
        self.polled_at = Instant::now();
        if matches!(&self.notice, Some(n) if n.kind == NoticeKind::Error) {
            self.notice = None;
        }

        let current = self.playback.as_ref().and_then(|p| p.track.clone());
        if let Some(track) = &current {
            if previous.as_deref() != Some(track.id.as_str()) {
                self.radio_played.insert(track.id.clone());
                if self.follow {
                    self.set_seed(track.clone(), cx);
                }
            }
            if let Some(url) = self.playback.as_ref().and_then(|p| p.large_art.clone()) {
                self.want_art(url, cx);
            }
        }
        if !self.volume_dragging
            && let Some(v) = self.playback.as_ref().and_then(|p| p.device.as_ref()?.volume_percent)
        {
            self.volume.update(cx, |s, cx| {
                if s.values().first().map(|x| x.round() as u8) != Some(v) {
                    s.set_value(vec![v as f64], cx);
                }
            });
        }
        self.sync_seek(cx);
        self.radio_tick(cx);
        self.activate_local_if_idle(cx);
        cx.notify();
    }

    /// Progress, advanced locally between polls so the bar moves smoothly.
    pub fn progress_ms(&self) -> u64 {
        let Some(p) = &self.playback else { return 0 };
        let duration = p.track.as_ref().map_or(0, |t| t.duration_ms);
        let elapsed = if p.is_playing { self.polled_at.elapsed().as_millis() as u64 } else { 0 };
        (p.progress_ms + elapsed).min(duration)
    }

    /// Keep the seek slider on the playing track; rebuild it when the
    /// duration changes because a slider's range is fixed at creation.
    fn sync_seek(&mut self, cx: &mut Context<Self>) {
        let track = self.playback.as_ref().and_then(|p| p.track.as_ref());
        let id = track.map(|t| t.id.clone());
        if id != self.seek_for {
            self.seek_for = id;
            self.seeking = false;
            let max = track.map_or(1.0, |t| (t.duration_ms as f64 / 1000.0).max(1.0));
            let disabled = track.is_none();
            self.seek = cx.new(|_| Slider::new("Seek", 0.0, 0.0, max, 1.0).disabled(disabled));
            self.seek_subscriptions = vec![
                cx.subscribe(&self.seek, |this, _, _: &InteractionStarted, _| this.seeking = true),
                cx.subscribe(&self.seek, |this, _, event: &InteractionEnded, cx| {
                    this.seeking = false;
                    let ms = (event.0.first().copied().unwrap_or(0.0) * 1000.0) as u64;
                    if let Some(p) = this.playback.as_mut() {
                        p.progress_ms = ms;
                        this.polled_at = Instant::now();
                    }
                    this.spotify_action(cx, move |s| s.seek(ms));
                }),
            ];
        }
        if !self.seeking {
            let secs = (self.progress_ms() / 1000) as f64;
            self.seek.update(cx, |s, cx| {
                if s.values().first() != Some(&secs) {
                    s.set_value(vec![secs], cx);
                }
            });
        }
    }

    // ----- commands -----------------------------------------------------

    /// Run a Spotify call in the background, then poll for the new state.
    pub fn spotify_action(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&SpotifyClient) -> Result<(), SpotifyError> + Send + 'static,
    ) {
        if self.demo {
            self.notice = Some(Notice {
                kind: NoticeKind::Info,
                message: "Demo mode: sign in to Spotify to control playback".into(),
            });
            cx.notify();
            return;
        }
        let Some(core) = &self.core else { return };
        let spotify = core.spotify.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { action(&spotify) }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.poll_now = true,
                    Err(e) => this.handle_error(e, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn handle_error(&mut self, error: SpotifyError, cx: &mut Context<Self>) {
        log::warn!("{error}");
        if matches!(error, SpotifyError::SignedOut) {
            self.sign_out(cx);
            self.setup_error = Some("Your Spotify session expired. Sign in again.".into());
            return;
        }
        self.notice = Some(Notice { kind: NoticeKind::Error, message: error.to_string() });
        cx.notify();
    }

    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        let playing = self.playback.as_ref().is_some_and(|p| p.is_playing);
        let progress = self.progress_ms();
        if let Some(p) = self.playback.as_mut() {
            // Optimistic: flip now, the next poll confirms.
            p.progress_ms = progress;
            p.is_playing = !playing;
            self.polled_at = Instant::now();
        }
        if playing {
            self.spotify_action(cx, |s| s.pause());
        } else {
            self.spotify_action(cx, |s| s.resume());
        }
        cx.notify();
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        self.spotify_action(cx, |s| s.next());
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        self.spotify_action(cx, |s| s.previous());
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        let on = !self.playback.as_ref().is_some_and(|p| p.shuffle);
        if let Some(p) = self.playback.as_mut() {
            p.shuffle = on;
        }
        self.spotify_action(cx, move |s| s.set_shuffle(on));
    }

    pub fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        let mode = self.playback.as_ref().map_or(RepeatMode::Off, |p| p.repeat).next();
        if let Some(p) = self.playback.as_mut() {
            p.repeat = mode;
        }
        self.spotify_action(cx, move |s| s.set_repeat(mode));
    }

    pub fn play_track(&mut self, track: &Track, cx: &mut Context<Self>) {
        let uris = vec![track.uri.clone()];
        self.radio_queued_for = None;
        self.spotify_action(cx, move |s| s.play_tracks(&uris));
    }

    pub fn queue_track(&mut self, track: &Track, cx: &mut Context<Self>) {
        let uri = track.uri.clone();
        let name = track.name.clone();
        self.notice = Some(Notice {
            kind: NoticeKind::Info,
            message: format!("Added “{name}” to the queue"),
        });
        self.spotify_action(cx, move |s| s.add_to_queue(&uri));
    }

    /// Play every related track that is on Spotify, in ranked order, and pin
    /// the seed so the list stays put while it plays.
    pub fn play_all_related(&mut self, cx: &mut Context<Self>) {
        let Some(report) = &self.report else { return };
        let uris: Vec<String> = report
            .tracks
            .iter()
            .filter_map(|r| r.track())
            .map(|t| t.uri.clone())
            .take(100)
            .collect();
        if uris.is_empty() {
            return;
        }
        self.set_follow(false, cx);
        self.spotify_action(cx, move |s| s.play_tracks(&uris));
        cx.notify();
    }

    pub fn transfer_to(&mut self, device: &Device, cx: &mut Context<Self>) {
        let Some(id) = device.id.clone() else { return };
        self.spotify_action(cx, move |s| s.transfer(&id, true));
    }

    pub fn set_follow(&mut self, follow: bool, cx: &mut Context<Self>) {
        self.follow = follow;
        let value = if follow { "follow" } else { "pinned" };
        self.seed_mode.update(cx, |control, cx| control.set_value(Some(value.into()), cx));
        if follow && let Some(track) = self.playback.as_ref().and_then(|p| p.track.clone()) {
            self.set_seed(track, cx);
        }
        cx.notify();
    }

    pub fn set_radio(&mut self, on: bool, cx: &mut Context<Self>) {
        self.radio = on;
        self.radio_queued_for = None;
        self.notice = on.then(|| Notice {
            kind: NoticeKind::Info,
            message: "Radio on: the top related track is queued before each song ends".into(),
        });
        self.radio_tick(cx);
        cx.notify();
    }

    /// Queue the best related track we have not played yet, once per song,
    /// shortly before the song ends.
    fn radio_tick(&mut self, cx: &mut Context<Self>) {
        if !self.radio {
            return;
        }
        let Some(track) = self.playback.as_ref().and_then(|p| p.track.clone()) else { return };
        let remaining = track.duration_ms.saturating_sub(self.progress_ms());
        if remaining > RADIO_LEAD_MS || self.radio_queued_for.as_deref() == Some(track.id.as_str())
        {
            return;
        }
        let Some(next) = self.report.as_ref().and_then(|r| {
            r.tracks
                .iter()
                .filter_map(|t| t.track())
                .find(|t| t.id != track.id && !self.radio_played.contains(&t.id))
                .cloned()
        }) else {
            return;
        };
        self.radio_queued_for = Some(track.id);
        self.radio_played.insert(next.id.clone());
        let uri = next.uri.clone();
        self.notice = Some(Notice {
            kind: NoticeKind::Info,
            message: format!("Radio queued “{}”", next.name),
        });
        self.spotify_action(cx, move |s| s.add_to_queue(&uri));
    }

    // ----- related tracks -----------------------------------------------

    /// Show related tracks for `track` without playing it, and stop following.
    pub fn explore(&mut self, track: Track, cx: &mut Context<Self>) {
        self.set_follow(false, cx);
        self.set_seed(track, cx);
    }

    pub fn refresh_related(&mut self, cx: &mut Context<Self>) {
        if let (Some(core), Some(seed)) = (&self.core, self.seed.clone()) {
            core.engine.invalidate(&Seed::from(&seed));
            self.seed = None;
            self.set_seed(seed, cx);
        }
    }

    pub fn set_seed(&mut self, track: Track, cx: &mut Context<Self>) {
        if self.seed.as_ref().is_some_and(|s| s.id == track.id) {
            return;
        }
        let Some(core) = &self.core else { return };
        let engine = core.engine.clone();
        self.history.retain(|t| t.id != track.id);
        self.history.insert(0, track.clone());
        self.history.truncate(HISTORY_LEN);
        if let Some(url) = track.album_art.clone() {
            self.want_art(url, cx);
        }
        self.seed = Some(track.clone());
        self.report = None;
        self.selected = None;
        self.loading_related = true;
        self.resolving = false;
        self.related_generation += 1;
        let generation = self.related_generation;
        let options = FusionOptions::from_config(&self.config);
        let demo = self.demo;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let seed = Seed::from(&track);
            let report = {
                let engine = engine.clone();
                cx.background_spawn(async move { engine.related(&seed, &options) }).await
            };
            let to_resolve: Vec<(usize, related_core::RelatedTrack)> = if demo {
                Vec::new()
            } else {
                report
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.spotify == Resolution::Pending)
                    .map(|(i, t)| (i, t.clone()))
                    .collect()
            };
            let current = this
                .update(cx, |this, cx| {
                    if this.related_generation != generation {
                        return false;
                    }
                    for url in report.tracks.iter().filter_map(art_url) {
                        this.want_art(url, cx);
                    }
                    this.report = Some(report);
                    this.loading_related = false;
                    this.resolving = !to_resolve.is_empty();
                    this.list_scroll.scroll_to_item(0, gpui_pre::ScrollStrategy::Top);
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !current {
                return;
            }

            for batch in to_resolve.chunks(RESOLVE_BATCH) {
                let (indices, tracks): (Vec<usize>, Vec<_>) = batch.iter().cloned().unzip();
                let resolved = {
                    let engine = engine.clone();
                    cx.background_spawn(async move { engine.resolve_many(&tracks) }).await
                };
                let current = this
                    .update(cx, |this, cx| {
                        if this.related_generation != generation {
                            return false;
                        }
                        if let Some(report) = this.report.as_mut() {
                            for (i, resolution) in indices.into_iter().zip(resolved) {
                                if let Some(slot) = report.tracks.get_mut(i) {
                                    slot.spotify = resolution;
                                }
                            }
                            let urls: Vec<String> =
                                report.tracks.iter().filter_map(art_url).collect();
                            for url in urls {
                                this.want_art(url, cx);
                            }
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
            }
            cx.background_spawn(async move { engine.save_cache() }).await;
            this.update(cx, |this, cx| {
                if this.related_generation == generation {
                    this.resolving = false;
                    this.radio_tick(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn select(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let len = self.report.as_ref().map_or(0, |r| r.tracks.len());
        self.selected = index.filter(|i| *i < len);
        if let Some(i) = self.selected {
            self.list_scroll.scroll_to_item(i, gpui_pre::ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.report.as_ref().map_or(0, |r| r.tracks.len());
        if len == 0 {
            return;
        }
        let next = match self.selected {
            None => 0,
            Some(i) => (i as isize + delta).clamp(0, len as isize - 1) as usize,
        };
        self.select(Some(next), cx);
    }

    pub fn selected_track(&self) -> Option<Track> {
        let report = self.report.as_ref()?;
        report.tracks.get(self.selected?)?.track().cloned()
    }

    /// The selected row as a seed, even when it is not on Spotify: the
    /// sources do not need Spotify to find its neighbours.
    pub fn selected_seed(&self) -> Option<Track> {
        let report = self.report.as_ref()?;
        report.tracks.get(self.selected?).map(seed_track)
    }

    // ----- search -------------------------------------------------------

    fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.search_query = query.trim().to_owned();
        self.search_generation += 1;
        let generation = self.search_generation;
        if self.search_query.is_empty() {
            self.search_results.clear();
            self.searching = false;
            cx.notify();
            return;
        }
        let Some(core) = &self.core else { return };
        let spotify = core.spotify.clone();
        let q = self.search_query.clone();
        self.searching = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { spotify.search_tracks(&q, 10) }).await;
            this.update(cx, |this, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.searching = false;
                match result {
                    Ok(tracks) => {
                        for url in tracks.iter().filter_map(|t| t.album_art.clone()) {
                            this.want_art(url, cx);
                        }
                        this.search_results = tracks;
                    }
                    Err(e) => this.handle_error(e, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ----- album art ----------------------------------------------------

    pub fn want_art(&mut self, url: String, cx: &mut Context<Self>) {
        if !self.art.begin(&url) {
            return;
        }
        let Some(core) = &self.core else { return };
        let agent = core.agent.clone();
        cx.spawn(async move |this, cx| {
            let fetch_url = url.clone();
            let bytes = cx.background_spawn(async move { images::fetch(&agent, &fetch_url) }).await;
            this.update(cx, |this, cx| {
                this.art.finish(url, bytes);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_dashboard(&self, cx: &mut App) {
        cx.open_url("https://developer.spotify.com/dashboard");
    }
}

/// The best thumbnail for a related row: Spotify's cover once resolved,
/// otherwise the source's own.
pub fn art_url(t: &related_core::RelatedTrack) -> Option<String> {
    t.track().and_then(|s| s.album_art.clone()).or_else(|| t.art_url.clone())
}

/// A related row as a `Track` to explore from: the Spotify track when
/// resolved, otherwise one built from the source's own metadata.
pub fn seed_track(row: &related_core::RelatedTrack) -> Track {
    row.track().cloned().unwrap_or_else(|| Track {
        id: format!("related:{}", row.key),
        uri: String::new(),
        name: row.title.clone(),
        artists: vec![row.artist.clone()],
        album: row.album.clone().unwrap_or_default(),
        album_art: row.art_url.clone(),
        duration_ms: 0,
        isrc: row.isrc.clone(),
    })
}
