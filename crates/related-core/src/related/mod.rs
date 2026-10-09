//! Related-track sources and the fusion that ranks their answers.

mod deezer;
mod fusion;
mod lastfm;
mod listenbrainz;

use serde::{Deserialize, Serialize};

pub use deezer::Deezer;
pub use fusion::{FusionOptions, RelatedTrack, Resolution, SourceHit, fuse};
pub use lastfm::LastFm;
pub use listenbrainz::ListenBrainz;

use crate::model::Track;

/// The track we want neighbours for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    pub title: String,
    pub artist: String,
    pub isrc: Option<String>,
    pub spotify_id: Option<String>,
}

impl From<&Track> for Seed {
    fn from(t: &Track) -> Self {
        Self {
            title: t.name.clone(),
            artist: t.primary_artist().to_owned(),
            isrc: t.isrc.clone(),
            spotify_id: Some(t.id.clone()),
        }
    }
}

impl Seed {
    pub fn key(&self) -> String {
        crate::normalize::track_key(&self.artist, &self.title)
    }
}

/// One suggestion from one source, in that source's own catalog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidate {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub isrc: Option<String>,
    pub art_url: Option<String>,
    /// The source's own similarity, scaled to 0..=1 when it reports one.
    pub similarity: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SourceId {
    LastFm,
    ListenBrainz,
    Deezer,
}

impl SourceId {
    pub const ALL: [SourceId; 3] = [SourceId::LastFm, SourceId::ListenBrainz, SourceId::Deezer];

    pub fn label(self) -> &'static str {
        match self {
            Self::LastFm => "Last.fm",
            Self::ListenBrainz => "ListenBrainz",
            Self::Deezer => "Deezer",
        }
    }

    /// What kind of signal the source contributes, for the UI.
    pub fn description(self) -> &'static str {
        match self {
            Self::LastFm => "Tracks scrobbled by the same listeners",
            Self::ListenBrainz => "Tracks played in the same listening sessions",
            Self::Deezer => "Artist radio built from similar artists",
        }
    }
}

/// A related-track source.
pub trait Provider: Send + Sync {
    fn id(&self) -> SourceId;
    /// Up to `limit` suggestions, best first.
    fn similar(&self, seed: &Seed, limit: usize) -> anyhow::Result<Vec<Candidate>>;
}
