//! Merge several ranked lists into one.
//!
//! Each source ranks its own catalog with its own scale, so raw scores are not
//! comparable. Fusion uses ranks instead (weighted reciprocal-rank fusion):
//! a track's score is the sum, over the sources that suggested it, of
//! `weight * (K + 1) / (K + rank + 1)`. A track two sources agree on beats a
//! track only one source ranked slightly higher, which is exactly the signal
//! a single-vendor recommender cannot give you.

use std::collections::HashMap;

use super::{Candidate, Seed, SourceId};
use crate::model::Track;
use crate::normalize::{artist_key, title_key, track_key};

/// Smoothing constant. Lower values favour each source's top picks more.
const K: f64 = 10.0;

#[derive(Clone, Debug, PartialEq)]
pub struct FusionOptions {
    pub weights: HashMap<SourceId, f64>,
    pub max_per_artist: usize,
    pub limit: usize,
}

impl Default for FusionOptions {
    fn default() -> Self {
        Self {
            weights: HashMap::from([
                (SourceId::LastFm, 1.0),
                (SourceId::ListenBrainz, 1.0),
                (SourceId::Deezer, 0.6),
            ]),
            max_per_artist: 2,
            limit: 40,
        }
    }
}

impl FusionOptions {
    pub fn from_config(config: &crate::Config) -> Self {
        Self {
            weights: HashMap::from([
                (SourceId::LastFm, config.sources.lastfm_weight),
                (SourceId::ListenBrainz, config.sources.listenbrainz_weight),
                (SourceId::Deezer, config.sources.deezer_weight),
            ]),
            max_per_artist: config.max_per_artist.max(1),
            limit: config.list_size.max(1),
        }
    }

    fn weight(&self, source: SourceId) -> f64 {
        self.weights.get(&source).copied().unwrap_or(1.0)
    }
}

/// Where one source ranked a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceHit {
    pub source: SourceId,
    /// Zero-based rank in that source's list.
    pub rank: usize,
    pub similarity: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelatedTrack {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub isrc: Option<String>,
    pub art_url: Option<String>,
    pub hits: Vec<SourceHit>,
    /// Fused score scaled so the best track is 1.0.
    pub score: f64,
    /// The playable Spotify track, once resolved.
    pub spotify: Resolution,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Resolution {
    #[default]
    Pending,
    Found(Track),
    NotOnSpotify,
}

impl RelatedTrack {
    pub fn track(&self) -> Option<&Track> {
        match &self.spotify {
            Resolution::Found(t) => Some(t),
            _ => None,
        }
    }

    pub fn has_source(&self, source: SourceId) -> bool {
        self.hits.iter().any(|h| h.source == source)
    }
}

pub fn fuse(
    seed: &Seed,
    lists: &[(SourceId, Vec<Candidate>)],
    opts: &FusionOptions,
) -> Vec<RelatedTrack> {
    let seed_key = seed.key();
    let seed_artist = artist_key(&seed.artist);
    let seed_title = title_key(&seed.title);

    let mut by_key: HashMap<String, (RelatedTrack, f64)> = HashMap::new();
    for (source, candidates) in lists {
        let weight = opts.weight(*source);
        if weight <= 0.0 {
            continue;
        }
        for (rank, c) in candidates.iter().enumerate() {
            let key = track_key(&c.artist, &c.title);
            let title = title_key(&c.title);
            if key == seed_key
                || title.is_empty()
                // Other versions of the seed itself: remixes, live takes, edits.
                || (artist_key(&c.artist) == seed_artist && title.starts_with(&seed_title))
            {
                continue;
            }
            let entry = by_key.entry(key.clone()).or_insert_with(|| {
                (
                    RelatedTrack {
                        key,
                        title: c.title.clone(),
                        artist: c.artist.clone(),
                        album: None,
                        isrc: None,
                        art_url: None,
                        hits: Vec::new(),
                        score: 0.0,
                        spotify: Resolution::Pending,
                    },
                    0.0,
                )
            });
            let (track, raw) = entry;
            if track.has_source(*source) {
                // A source listed the same recording twice (different editions).
                continue;
            }
            track.album = track.album.take().or_else(|| c.album.clone());
            track.isrc = track.isrc.take().or_else(|| c.isrc.clone());
            track.art_url = track.art_url.take().or_else(|| c.art_url.clone());
            track.hits.push(SourceHit { source: *source, rank, similarity: c.similarity });
            *raw += weight * (K + 1.0) / (K + rank as f64 + 1.0);
        }
    }

    let mut ranked: Vec<(RelatedTrack, f64)> = by_key.into_values().collect();
    ranked.sort_by(|(a, ra), (b, rb)| {
        rb.partial_cmp(ra).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.key.cmp(&b.key))
    });
    let best = ranked.first().map(|(_, r)| *r).unwrap_or(1.0).max(f64::EPSILON);

    let mut per_artist: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(opts.limit);
    for (mut track, raw) in ranked {
        let count = per_artist.entry(artist_key(&track.artist)).or_default();
        if *count >= opts.max_per_artist {
            continue;
        }
        *count += 1;
        track.score = raw / best;
        track.hits.sort_by_key(|h| h.source);
        out.push(track);
        if out.len() == opts.limit {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(artist: &str, title: &str) -> Candidate {
        Candidate { artist: artist.into(), title: title.into(), ..Default::default() }
    }

    fn seed() -> Seed {
        Seed {
            title: "Blinding Lights".into(),
            artist: "The Weeknd".into(),
            isrc: None,
            spotify_id: None,
        }
    }

    #[test]
    fn agreement_beats_a_single_higher_rank() {
        let lists = vec![
            (SourceId::LastFm, vec![c("A", "Solo"), c("B", "Shared")]),
            (SourceId::ListenBrainz, vec![c("C", "Other"), c("B", "Shared")]),
        ];
        let out = fuse(&seed(), &lists, &FusionOptions::default());
        assert_eq!(out[0].title, "Shared");
        assert_eq!(out[0].score, 1.0);
        assert_eq!(out[0].hits.len(), 2);
    }

    #[test]
    fn drops_the_seed_and_its_versions() {
        let lists = vec![(
            SourceId::Deezer,
            vec![
                c("The Weeknd", "Blinding Lights"),
                c("Weeknd", "Blinding Lights (Chromatics Remix)"),
                c("The Weeknd", "Save Your Tears"),
            ],
        )];
        let out = fuse(&seed(), &lists, &FusionOptions::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "Save Your Tears");
    }

    #[test]
    fn merges_spellings_and_caps_artists() {
        let lists = vec![
            (
                SourceId::LastFm,
                vec![
                    c("Dua Lipa", "Don't Start Now"),
                    c("Dua Lipa", "Levitating"),
                    c("Dua Lipa", "Physical"),
                ],
            ),
            (SourceId::ListenBrainz, vec![c("Dua Lipa", "Don’t Start Now")]),
        ];
        let opts = FusionOptions { max_per_artist: 2, ..Default::default() };
        let out = fuse(&seed(), &lists, &opts);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].hits.len(), 2);
    }

    #[test]
    fn zero_weight_disables_a_source() {
        let lists = vec![(SourceId::Deezer, vec![c("A", "x")])];
        let mut opts = FusionOptions::default();
        opts.weights.insert(SourceId::Deezer, 0.0);
        assert!(fuse(&seed(), &lists, &opts).is_empty());
    }
}
