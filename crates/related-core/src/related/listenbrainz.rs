//! ListenBrainz session-based similarity: recordings that people play in the
//! same listening sessions. Open data, no key. Recordings are identified by
//! MusicBrainz IDs, so the seed is first looked up on MusicBrainz, by ISRC
//! when Spotify provides one and by title and artist otherwise.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;
use ureq::Agent;

use super::{Candidate, Provider, Seed, SourceId};
use crate::normalize::{artist_key, title_key};

const MUSICBRAINZ: &str = "https://musicbrainz.org/ws/2";
const SIMILAR: &str = "https://labs.api.listenbrainz.org/similar-recordings/json";
/// All-time sessions, long window. The dataset with the widest coverage.
const ALGORITHM: &str = "session_based_days_7500_session_300_contribution_5_threshold_15_limit_50_skip_30";
/// Many recordings of a song have no session data; try a few before giving up.
const MAX_MBIDS: usize = 4;

pub struct ListenBrainz {
    agent: Agent,
    /// MusicBrainz allows one request per second per client.
    last_mb_call: Mutex<Option<Instant>>,
}

impl ListenBrainz {
    pub fn new(agent: Agent) -> Self {
        Self { agent, last_mb_call: Mutex::new(None) }
    }

    fn musicbrainz(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        {
            let mut last = self.last_mb_call.lock().unwrap();
            if let Some(t) = *last {
                let wait = Duration::from_millis(1100).saturating_sub(t.elapsed());
                std::thread::sleep(wait);
            }
            *last = Some(Instant::now());
        }
        let mut req = self.agent.get(format!("{MUSICBRAINZ}{path}")).query("fmt", "json");
        for (k, v) in query {
            req = req.query(*k, *v);
        }
        Ok(req.call()?.body_mut().read_json()?)
    }

    fn recording_mbids(&self, seed: &Seed) -> Result<Vec<String>> {
        if let Some(isrc) = &seed.isrc {
            let v = self.musicbrainz(&format!("/isrc/{isrc}"), &[])?;
            let ids = ids(&v);
            if !ids.is_empty() {
                return Ok(ids);
            }
        }
        let query = format!(
            "recording:\"{}\" AND artist:\"{}\"",
            lucene_escape(&seed.title),
            lucene_escape(&seed.artist)
        );
        let v = self.musicbrainz("/recording", &[("query", &query), ("limit", "8")])?;
        let want_title = title_key(&seed.title);
        let want_artist = artist_key(&seed.artist);
        Ok(v.get("recordings")
            .and_then(Value::as_array)
            .map(|recs| {
                recs.iter()
                    .filter(|r| {
                        let title = r.get("title").and_then(Value::as_str).unwrap_or("");
                        let artist =
                            r.pointer("/artist-credit/0/name").and_then(Value::as_str).unwrap_or("");
                        title_key(title) == want_title && artist_key(artist) == want_artist
                    })
                    .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }

    fn similar_to(&self, mbid: &str) -> Result<Vec<Value>> {
        let v: Value = self
            .agent
            .get(SIMILAR)
            .query("recording_mbids", mbid)
            .query("algorithm", ALGORITHM)
            .call()?
            .body_mut()
            .read_json()?;
        Ok(v.as_array().cloned().unwrap_or_default())
    }
}

impl Provider for ListenBrainz {
    fn id(&self) -> SourceId {
        SourceId::ListenBrainz
    }

    fn similar(&self, seed: &Seed, limit: usize) -> Result<Vec<Candidate>> {
        for mbid in self.recording_mbids(seed)?.into_iter().take(MAX_MBIDS) {
            let rows = self.similar_to(&mbid)?;
            if !rows.is_empty() {
                let mut out = parse_rows(&rows);
                out.truncate(limit);
                return Ok(out);
            }
        }
        Ok(Vec::new())
    }
}

fn ids(v: &Value) -> Vec<String> {
    v.get("recordings")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|r| r.get("id")?.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn parse_rows(rows: &[Value]) -> Vec<Candidate> {
    let top = rows
        .iter()
        .filter_map(|r| r.get("score").and_then(Value::as_f64))
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let mut out: Vec<Candidate> = rows
        .iter()
        .filter_map(|r| {
            let art_url = match (
                r.get("caa_release_mbid").and_then(Value::as_str),
                r.get("caa_id").and_then(Value::as_u64),
            ) {
                (Some(release), Some(id)) => {
                    Some(format!("https://coverartarchive.org/release/{release}/{id}-250.jpg"))
                }
                _ => None,
            };
            Some(Candidate {
                title: r.get("recording_name")?.as_str()?.to_owned(),
                artist: r.get("artist_credit_name")?.as_str()?.to_owned(),
                album: r.get("release_name").and_then(Value::as_str).map(str::to_owned),
                isrc: None,
                art_url,
                similarity: r.get("score").and_then(Value::as_f64).map(|s| s / top),
            })
        })
        .collect();
    out.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn lucene_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            let special = "+-&|!(){}[]^\"~*?:\\/".contains(c);
            special.then_some('\\').into_iter().chain(std::iter::once(c))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_rows_and_scales_scores() {
        let rows = vec![
            json!({"recording_name": "Levitating", "artist_credit_name": "Dua Lipa", "score": 600,
                   "release_name": "Future Nostalgia", "caa_release_mbid": "r", "caa_id": 1}),
            json!({"recording_name": "Don’t Start Now", "artist_credit_name": "Dua Lipa", "score": 1200}),
        ];
        let c = parse_rows(&rows);
        assert_eq!(c[0].title, "Don’t Start Now");
        assert_eq!(c[0].similarity, Some(1.0));
        assert_eq!(c[1].similarity, Some(0.5));
        assert_eq!(c[1].art_url.as_deref(), Some("https://coverartarchive.org/release/r/1-250.jpg"));
    }

    #[test]
    fn escapes_lucene() {
        assert_eq!(lucene_escape("AC/DC: \"Hi\""), "AC\\/DC\\: \\\"Hi\\\"");
    }
}
