//! Run the built-in Connect device on its own and print its status.
//!
//! ```sh
//! # First run: a token with the `streaming` scope (credentials are cached after).
//! SPOTIFY_TOKEN=... cargo run -p local-player --example device
//! cargo run -p local-player --example device
//! ```

use std::time::{Duration, Instant};

use local_player::{LocalPlayer, LocalPlayerConfig, Status};

fn main() {
    env_logger_lite();
    let dir = std::env::var_os("LOCAL_PLAYER_DIR")
        .map(Into::into)
        .unwrap_or_else(|| std::env::temp_dir().join("local-player-example"));
    let config = LocalPlayerConfig {
        device_name: "local-player example".into(),
        credentials_dir: dir,
        audio_cache_dir: None,
        bitrate: 160,
        initial_volume: 50,
        normalise: false,
    };
    let player = LocalPlayer::start(config, std::env::var("SPOTIFY_TOKEN").ok()).expect("start");
    let started = Instant::now();
    let mut last = None;
    while started.elapsed() < Duration::from_secs(60) {
        let status = player.status();
        if last.as_ref() != Some(&status) {
            println!("{:>5.1}s  {status:?}", started.elapsed().as_secs_f32());
            last = Some(status.clone());
        }
        if matches!(status, Status::NeedsAuthorization { reason: Some(_) } | Status::Failed(_)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Print librespot's warnings without pulling in a logger dependency.
fn env_logger_lite() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::Level::Info
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("[{}] {}: {}", r.level(), r.target(), r.args());
            }
        }
        fn flush(&self) {}
    }
    let _ = log::set_logger(&Stderr).map(|()| log::set_max_level(log::LevelFilter::Info));
}
