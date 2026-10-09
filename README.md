# spotify-related

A cross-platform Spotify player built around one feature: a good list of related songs for whatever is playing.

Spotify's own "related" data is a single black box, and its public API for it was shut off for new apps in 2024. spotify-related asks open music-data services instead, merges their answers, shows you where each pick came from, and plays the result on your Spotify account.

![The main window in demo mode](docs/screenshot.png)

The UI is written in Rust with [GPUI](https://www.gpui.rs) and the [mkit](https://github.com/s4njee/mkit) component kit (shadcn dark theme, Geist type). It runs on macOS, Windows and Linux.

## How the related list is built

For the seed song, three sources are asked in parallel:

| Source | Signal | Key needed |
| --- | --- | --- |
| **Last.fm** `track.getSimilar` | Tracks scrobbled by the same listeners. The strongest track-level signal. Falls back to similar artists' top tracks for rare songs. | Free API key |
| **ListenBrainz** similar recordings | Tracks played in the same listening sessions (open data). The seed is matched on MusicBrainz by ISRC, or by title and artist. | No |
| **Deezer** artist radio | A mix of the seed artist and similar artists. Coarser, but covers new and regional releases. | No |

The lists are merged with weighted reciprocal-rank fusion: each source contributes `weight × 11 / (10 + rank)` to a track's score, so a song two sources agree on beats a song one source ranked slightly higher. Titles and artists are normalised across catalogs ("Song - 2011 Remaster", "Song (feat. X)", accents and curly quotes), other versions of the seed itself are dropped, and no artist gets more than two spots. Each pick is then found on Spotify, by ISRC when the source has one and by a verified title-and-artist search otherwise.

Every row shows which sources suggested it (hover a pill for that source's rank and similarity) and a match bar for the fused score.

You can try the pipeline without the UI or a Spotify account:

```sh
cargo run -p related-core --example related -- "The Weeknd" "Blinding Lights"
LASTFM_API_KEY=... cargo run -p related-core --example related -- "Kate Bush" "Running Up That Hill"
```

## What you can do

- **Follow now playing** (default): the list updates whenever the song changes, wherever it is playing.
- **Explore** any song (search result, history entry or related row) to see its neighbours without playing it. This pins the list.
- **Play**, **Add to queue**, or **Play all** related tracks in ranked order.
- **Radio**: shortly before each song ends, the best related track you have not heard this session is added to the queue, so playback keeps walking the related graph.
- **Built-in player**: the app is its own Spotify Connect device (via [librespot](https://github.com/librespot-org/librespot)), so music plays right here. It also shows up in Spotify on your phone, and you can hand playback to any other device from the sidebar.
- Transport, seek, volume, shuffle, repeat, and switching between Spotify Connect devices.

Keyboard, in the related list: <kbd>↑</kbd>/<kbd>↓</kbd> (or <kbd>j</kbd>/<kbd>k</kbd>) select, <kbd>Enter</kbd> play, <kbd>Q</kbd> queue, <kbd>E</kbd> explore, <kbd>Space</kbd> play/pause. Anywhere: <kbd>Cmd/Ctrl</kbd>+<kbd>←</kbd>/<kbd>→</kbd> previous/next, <kbd>Cmd/Ctrl</kbd>+<kbd>P</kbd> play/pause, <kbd>Cmd/Ctrl</kbd>+<kbd>R</kbd> refresh the list.

## Setup

You need **Spotify Premium**: Spotify allows playback control and third-party streaming only for Premium accounts.

1. Create an app at the [Spotify developer dashboard](https://developer.spotify.com/dashboard). Add the redirect URI `http://127.0.0.1:8898/callback` and tick **Web API**. Spotify keeps new apps in development mode, which only lets accounts listed under **User Management** sign in, so add your own account there.
2. Optionally get a free [Last.fm API key](https://www.last.fm/api/account/create). Without it the list uses ListenBrainz and Deezer only.
3. Run the app, paste the client ID (and Last.fm key), and choose **Sign in with Spotify**. Your browser opens Spotify's consent page; afterwards the app keeps a refresh token so you only do this once.

The built-in player then signs in as a Connect device named **spotify-related** using the same sign-in, and becomes the active device if nothing else is playing. If Spotify refuses your app's token for audio streaming, the Devices panel shows **Allow playback here**. That runs the same one-time browser approval the librespot CLI uses, with librespot's client ID and `http://127.0.0.1:5588/login`. After the first connection librespot keeps its own reusable credentials, so neither step repeats.

To look around first, run it with `--demo`: the related list is live for a sample song, but nothing plays.

```sh
cargo run --release -p spotify-related            # the app
cargo run --release -p spotify-related -- --demo  # no Spotify account needed
```

The app asks only for playback scopes (`user-read-playback-state`, `user-modify-playback-state`, `user-read-currently-playing`, and `streaming` for the built-in player). It never reads or changes your library or playlists.

### Configuration

Settings live in `config.toml` in the platform config directory (`~/.config/spotify-related/` on Linux, `~/Library/Application Support/spotify-related/` on macOS, `%APPDATA%\spotify-related\` on Windows). The setup screen writes it; you can also edit it by hand:

```toml
spotify_client_id = "your-client-id"
lastfm_api_key = "your-lastfm-key"   # optional
redirect_port = 8898                 # must match the redirect URI on your Spotify app
max_per_artist = 2                   # cap per artist in the related list
list_size = 40

[sources]                            # set a weight to 0 to turn a source off
lastfm_weight = 1.0
listenbrainz_weight = 1.0
deezer_weight = 0.6

[local_playback]                     # the built-in player
enabled = true
device_name = "spotify-related"      # how this computer appears in Spotify
bitrate = 320                        # 96, 160 or 320 kbps
normalise = false                    # even out loudness between tracks
audio_cache = true                   # keep up to 1 GB of recent audio on disk
```

`SPOTIFY_CLIENT_ID`, `LASTFM_API_KEY` and `SPOTIFY_REDIRECT_PORT` override the file. The refresh token is stored next to it in `token.json` (readable only by you on macOS and Linux), and the built-in player's credentials and volume in `librespot/`. Spotify matches and cached audio go in the platform cache directory. Signing out removes both sets of credentials. The built-in player honours `HTTPS_PROXY`.

## Building

Rust 1.92 or newer. GPUI and mkit are pinned to the versions mkit is tested with (`gpui-pre` 0.3.5, mkit `d5f1a07`).

- **macOS**: Xcode and its command line tools (`xcode-select --install`).
- **Windows**: the MSVC Rust toolchain with Visual Studio C++ Build Tools and a Windows SDK.
- **Linux**: a C toolchain, `pkg-config`, and the X11/Wayland, Vulkan and font development packages. On Ubuntu:

  ```sh
  sudo apt-get install build-essential clang pkg-config libxkbcommon-x11-dev libwayland-dev \
    libx11-xcb-dev libfontconfig-dev libssl-dev libvulkan1 libzstd-dev libglib2.0-dev libasound2-dev
  ```

Then `cargo build --release --locked -p spotify-related`. Use `--locked`: librespot 0.8's build script breaks with `vergen` 9.1, and the lockfile pins 9.0.6. `cargo test --workspace` runs the unit tests for parsing, normalisation, fusion and the OAuth callback.

The built-in player is the default `local-playback` feature. Build with `--no-default-features` for a smaller app that only controls other devices. To try the device on its own: `SPOTIFY_TOKEN=<token with streaming scope> cargo run -p local-player --example device`.

## Layout

```
crates/related-core   No UI. Config, Spotify Web API client (PKCE sign-in, Connect playback,
                      search), the Last.fm / ListenBrainz / Deezer providers, fusion, and the
                      engine that resolves picks to Spotify tracks. Synchronous; runs on
                      GPUI's background executor.
crates/local-player   The built-in player: a librespot Connect device on its own thread and
                      tokio runtime, with cached credentials, reconnects and status reporting.
crates/app            The GPUI window. player.rs holds state and async work; ui/ renders it
                      with mkit components (Button, IconButton, Slider, SearchField,
                      SegmentedControl, Badge, Tooltip, EmptyState, Progress).
```

## Roadmap

- More sources (for example Discogs styles or MusicBrainz tags) and per-source tuning in the UI.
- A settings screen for weights and the per-artist cap, and an mkit light theme toggle.
- Saving a related list as a Spotify playlist (needs one more scope).

## Credits

Related-track data from [Last.fm](https://www.last.fm/api), [ListenBrainz](https://listenbrainz.org) and [MusicBrainz](https://musicbrainz.org) (open data), and [Deezer](https://developers.deezer.com). Cover art from Spotify and the [Cover Art Archive](https://coverartarchive.org). Playback through [librespot](https://github.com/librespot-org/librespot) (MIT). The Geist and Geist Mono fonts are by Vercel under the SIL Open Font License (`crates/app/assets/fonts/OFL.txt`).
