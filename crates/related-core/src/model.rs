//! Plain data shared by the Spotify client, the engine and the UI.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A Spotify track, reduced to what the player shows and plays.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album: String,
    /// The smallest album image that is at least 64px, when Spotify sends one.
    pub album_art: Option<String>,
    pub duration_ms: u64,
    /// International Standard Recording Code. It lets the providers find the
    /// exact recording instead of guessing from the title.
    pub isrc: Option<String>,
}

impl Track {
    pub fn primary_artist(&self) -> &str {
        self.artists.first().map(String::as_str).unwrap_or("")
    }

    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }

    /// Parse a Spotify track object. Returns `None` for episodes, local files
    /// without a URI, and anything else that is not a playable track.
    pub fn from_json(v: &Value) -> Option<Self> {
        if v.get("type").and_then(Value::as_str).is_some_and(|t| t != "track") {
            return None;
        }
        let uri = v.get("uri")?.as_str()?.to_owned();
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| uri.rsplit(':').next().unwrap_or_default().to_owned());
        let artists = v
            .get("artists")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.get("name").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let album = v.get("album");
        let album_art = album
            .and_then(|a| a.get("images"))
            .and_then(Value::as_array)
            .and_then(|images| pick_image(images, 64));
        Some(Self {
            id,
            uri,
            name: v.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
            artists,
            album: album
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            album_art,
            duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
            isrc: v
                .get("external_ids")
                .and_then(|e| e.get("isrc"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    /// The largest album image URL, for the now-playing panel.
    pub fn large_art_from_json(v: &Value) -> Option<String> {
        let images = v.get("album")?.get("images")?.as_array()?;
        pick_image(images, 640)
    }
}

/// Spotify lists images largest first. Pick the smallest one at least `min`
/// pixels wide, or the largest if none is that big.
fn pick_image(images: &[Value], min: u64) -> Option<String> {
    let sized: Vec<(u64, &str)> = images
        .iter()
        .filter_map(|image| {
            let url = image.get("url").and_then(Value::as_str)?;
            Some((image.get("width").and_then(Value::as_u64).unwrap_or(0), url))
        })
        .collect();
    sized
        .iter()
        .filter(|(width, _)| *width >= min)
        .min_by_key(|(width, _)| *width)
        .or_else(|| sized.iter().max_by_key(|(width, _)| *width))
        .map(|(_, url)| (*url).to_owned())
}

/// A Spotify Connect device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: Option<String>,
    pub name: String,
    pub kind: String,
    pub is_active: bool,
    pub volume_percent: Option<u8>,
}

impl Device {
    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            id: v.get("id").and_then(Value::as_str).map(str::to_owned),
            name: v.get("name")?.as_str()?.to_owned(),
            kind: v.get("type").and_then(Value::as_str).unwrap_or("Unknown").to_owned(),
            is_active: v.get("is_active").and_then(Value::as_bool).unwrap_or(false),
            volume_percent: v
                .get("volume_percent")
                .and_then(Value::as_u64)
                .map(|x| x.min(100) as u8),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    Context,
    Track,
}

impl RepeatMode {
    pub fn as_api(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Context => "context",
            Self::Track => "track",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Context,
            Self::Context => Self::Track,
            Self::Track => Self::Off,
        }
    }
}

/// What `GET /me/player` reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackState {
    pub is_playing: bool,
    pub progress_ms: u64,
    pub track: Option<Track>,
    pub large_art: Option<String>,
    pub device: Option<Device>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

impl PlaybackState {
    pub fn from_json(v: &Value) -> Self {
        let item = v.get("item");
        Self {
            is_playing: v.get("is_playing").and_then(Value::as_bool).unwrap_or(false),
            progress_ms: v.get("progress_ms").and_then(Value::as_u64).unwrap_or(0),
            track: item.and_then(Track::from_json),
            large_art: item.and_then(Track::large_art_from_json),
            device: v.get("device").and_then(Device::from_json),
            shuffle: v.get("shuffle_state").and_then(Value::as_bool).unwrap_or(false),
            repeat: match v.get("repeat_state").and_then(Value::as_str) {
                Some("context") => RepeatMode::Context,
                Some("track") => RepeatMode::Track,
                _ => RepeatMode::Off,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_track_and_picks_art() {
        let v = json!({
            "type": "track",
            "id": "abc",
            "uri": "spotify:track:abc",
            "name": "Blinding Lights",
            "duration_ms": 200040,
            "artists": [{"name": "The Weeknd"}],
            "album": {"name": "After Hours", "images": [
                {"url": "big", "width": 640},
                {"url": "mid", "width": 300},
                {"url": "small", "width": 64}
            ]},
            "external_ids": {"isrc": "USUG11904206"}
        });
        let t = Track::from_json(&v).unwrap();
        assert_eq!(t.primary_artist(), "The Weeknd");
        assert_eq!(t.album_art.as_deref(), Some("small"));
        assert_eq!(t.isrc.as_deref(), Some("USUG11904206"));
        assert_eq!(Track::large_art_from_json(&v).as_deref(), Some("big"));
    }

    #[test]
    fn episodes_are_not_tracks() {
        let v = json!({"type": "episode", "uri": "spotify:episode:x", "name": "Pod"});
        assert!(Track::from_json(&v).is_none());
    }

    #[test]
    fn parses_playback_state() {
        let v = json!({
            "is_playing": true,
            "progress_ms": 1234,
            "shuffle_state": true,
            "repeat_state": "track",
            "device": {"id": "d1", "name": "Desk", "type": "Computer", "is_active": true, "volume_percent": 55},
            "item": null
        });
        let s = PlaybackState::from_json(&v);
        assert!(s.is_playing && s.shuffle);
        assert_eq!(s.repeat, RepeatMode::Track);
        assert_eq!(s.device.unwrap().volume_percent, Some(55));
        assert!(s.track.is_none());
    }
}
