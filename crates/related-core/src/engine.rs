//! Ask every configured source in parallel, fuse the answers, and map them
//! onto playable Spotify tracks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::config::{self, Config};
use crate::model::Track;
use crate::normalize::{artist_key, title_key};
use crate::related::{
    Deezer, FusionOptions, LastFm, ListenBrainz, Provider, RelatedTrack, Resolution, Seed, SourceId,
    fuse,
};
use crate::spotify::{SpotifyClient, SpotifyError};

/// Ask each source for more than we show; fusion and the artist cap discard many.
const PER_SOURCE: usize = 60;
const RESOLVE_WORKERS: usize = 4;
const REPORT_CACHE_SIZE: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct SourceStatus {
    pub source: SourceId,
    /// Number of suggestions, or why the source failed.
    pub outcome: Result<usize, String>,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelatedReport {
    pub seed: Seed,
    pub tracks: Vec<RelatedTrack>,
    pub sources: Vec<SourceStatus>,
}

pub struct RelatedEngine {
    providers: Vec<Box<dyn Provider>>,
    spotify: Arc<SpotifyClient>,
    resolved: Mutex<HashMap<String, Option<Track>>>,
    reports: Mutex<Vec<(String, RelatedReport)>>,
    cache_path: Option<PathBuf>,
}

impl RelatedEngine {
    pub fn new(config: &Config, spotify: Arc<SpotifyClient>) -> Self {
        let agent = crate::http::agent();
        let mut providers: Vec<Box<dyn Provider>> = Vec::new();
        if let Some(key) = &config.lastfm_api_key {
            providers.push(Box::new(LastFm::new(agent.clone(), key.clone())));
        }
        providers.push(Box::new(ListenBrainz::new(agent.clone())));
        providers.push(Box::new(Deezer::new(agent)));
        Self::with_providers(providers, spotify)
    }

    pub fn with_providers(providers: Vec<Box<dyn Provider>>, spotify: Arc<SpotifyClient>) -> Self {
        let cache_path = config::cache_dir().map(|d| d.join("resolved.json"));
        let resolved = cache_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            providers,
            spotify,
            resolved: Mutex::new(resolved),
            reports: Mutex::new(Vec::new()),
            cache_path,
        }
    }

    pub fn sources(&self) -> Vec<SourceId> {
        self.providers.iter().map(|p| p.id()).collect()
    }

    /// Related tracks for `seed`, best first, not yet resolved on Spotify.
    /// Results are cached per seed for the session.
    pub fn related(&self, seed: &Seed, opts: &FusionOptions) -> RelatedReport {
        let key = seed.key();
        if let Some((_, cached)) = self.reports.lock().unwrap().iter().find(|(k, _)| *k == key) {
            let mut report = cached.clone();
            for track in &mut report.tracks {
                track.spotify = self.cached_resolution(track);
            }
            return report;
        }

        let results: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .providers
                .iter()
                .map(|p| {
                    scope.spawn(move || {
                        let start = Instant::now();
                        let result = p.similar(seed, PER_SOURCE);
                        (p.id(), result, start.elapsed().as_millis())
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("provider thread panicked")).collect()
        });

        let mut lists = Vec::new();
        let mut sources = Vec::new();
        for (source, result, elapsed_ms) in results {
            let outcome = match result {
                Ok(list) => {
                    let n = list.len();
                    lists.push((source, list));
                    Ok(n)
                }
                Err(e) => {
                    log::warn!("{} failed: {e:#}", source.label());
                    Err(format!("{e:#}"))
                }
            };
            sources.push(SourceStatus { source, outcome, elapsed_ms });
        }
        let mut tracks = fuse(seed, &lists, opts);
        for track in &mut tracks {
            track.spotify = self.cached_resolution(track);
        }
        let report = RelatedReport { seed: seed.clone(), tracks, sources };

        // Only cache answers where at least one source worked.
        if report.sources.iter().any(|s| s.outcome.is_ok()) {
            let mut reports = self.reports.lock().unwrap();
            reports.push((key, report.clone()));
            if reports.len() > REPORT_CACHE_SIZE {
                reports.remove(0);
            }
        }
        report
    }

    /// Forget the cached report for `seed`, so the next call asks the sources again.
    pub fn invalidate(&self, seed: &Seed) {
        let key = seed.key();
        self.reports.lock().unwrap().retain(|(k, _)| *k != key);
    }

    fn cached_resolution(&self, track: &RelatedTrack) -> Resolution {
        match self.resolved.lock().unwrap().get(&track.key) {
            Some(Some(t)) => Resolution::Found(t.clone()),
            Some(None) => Resolution::NotOnSpotify,
            None => Resolution::Pending,
        }
    }

    /// Find each track on Spotify, a few at a time. Network failures leave a
    /// track `Pending` so a later call can retry it.
    pub fn resolve_many(&self, tracks: &[RelatedTrack]) -> Vec<Resolution> {
        let mut out = vec![Resolution::Pending; tracks.len()];
        for (chunk_out, chunk) in out.chunks_mut(RESOLVE_WORKERS).zip(tracks.chunks(RESOLVE_WORKERS)) {
            std::thread::scope(|scope| {
                let handles: Vec<_> =
                    chunk.iter().map(|t| scope.spawn(move || self.resolve(t))).collect();
                for (slot, h) in chunk_out.iter_mut().zip(handles) {
                    *slot = h.join().unwrap_or(Resolution::Pending);
                }
            });
        }
        out
    }

    pub fn resolve(&self, track: &RelatedTrack) -> Resolution {
        let cached = self.cached_resolution(track);
        if cached != Resolution::Pending {
            return cached;
        }
        match self.search_spotify(track) {
            Ok(found) => {
                self.resolved.lock().unwrap().insert(track.key.clone(), found.clone());
                found.map_or(Resolution::NotOnSpotify, Resolution::Found)
            }
            Err(e) => {
                log::debug!("resolving {} - {}: {e}", track.artist, track.title);
                Resolution::Pending
            }
        }
    }

    fn search_spotify(&self, track: &RelatedTrack) -> Result<Option<Track>, SpotifyError> {
        if let Some(isrc) = &track.isrc
            && let Some(hit) = self.spotify.search_tracks(&format!("isrc:{isrc}"), 1)?.into_iter().next()
        {
            return Ok(Some(hit));
        }
        let queries = [
            format!("{} {}", track.title, track.artist),
            format!("track:{} artist:{}", strip_quotes(&track.title), strip_quotes(&track.artist)),
        ];
        for query in queries {
            let hits = self.spotify.search_tracks(&query, 5)?;
            if let Some(hit) = best_match(track, hits) {
                return Ok(Some(hit));
            }
        }
        Ok(None)
    }

    pub fn save_cache(&self) {
        let Some(path) = &self.cache_path else { return };
        let bytes = serde_json::to_vec(&*self.resolved.lock().unwrap()).expect("cache serializes");
        if let Err(e) = config::write_private(path, &bytes) {
            log::warn!("could not save resolve cache: {e:#}");
        }
    }
}

fn strip_quotes(s: &str) -> String {
    s.replace('"', "")
}

/// Accept a search hit only if the artist matches and the title matches
/// exactly, or one title is the other plus a suffix.
fn best_match(want: &RelatedTrack, hits: Vec<Track>) -> Option<Track> {
    let want_artist = artist_key(&want.artist);
    let want_title = title_key(&want.title);
    let artist_ok = |t: &Track| t.artists.iter().any(|a| artist_key(a) == want_artist);
    let exact = hits.iter().position(|t| artist_ok(t) && title_key(&t.name) == want_title);
    let loose = || {
        hits.iter().position(|t| {
            let title = title_key(&t.name);
            artist_ok(t) && (title.starts_with(&want_title) || want_title.starts_with(&title))
        })
    };
    exact.or_else(loose).map(|i| hits[i].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spotify_track(name: &str, artist: &str) -> Track {
        Track {
            id: name.into(),
            uri: format!("spotify:track:{name}"),
            name: name.into(),
            artists: vec![artist.into()],
            album: String::new(),
            album_art: None,
            duration_ms: 0,
            isrc: None,
        }
    }

    fn related(artist: &str, title: &str) -> RelatedTrack {
        RelatedTrack {
            key: crate::normalize::track_key(artist, title),
            title: title.into(),
            artist: artist.into(),
            album: None,
            isrc: None,
            art_url: None,
            hits: vec![],
            score: 1.0,
            spotify: Resolution::Pending,
        }
    }

    #[test]
    fn prefers_exact_title_and_checks_artist() {
        let hits = vec![
            spotify_track("Levitating (feat. DaBaby)", "Dua Lipa"),
            spotify_track("Levitating", "Some Cover Band"),
            spotify_track("Levitating", "Dua Lipa"),
        ];
        let m = best_match(&related("Dua Lipa", "Levitating"), hits).unwrap();
        // "(feat. …)" is packaging, so the first hit is already an exact match.
        assert_eq!(m.artists[0], "Dua Lipa");
        assert!(best_match(&related("Dua Lipa", "Levitating"), vec![spotify_track("Levitating", "Other")]).is_none());
    }

    #[test]
    fn accepts_suffixed_titles() {
        let hits = vec![spotify_track("Hounds of Love - 2018 Remaster", "Kate Bush")];
        assert!(best_match(&related("Kate Bush", "Hounds of Love"), hits).is_some());
    }
}
