//! Last.fm `track.getSimilar`: collaborative filtering over scrobbles. The
//! best single source for "people who play this also play", but it needs a
//! free API key from https://www.last.fm/api/account/create.

use anyhow::{Result, bail};
use serde_json::Value;
use ureq::Agent;

use super::{Candidate, Provider, Seed, SourceId};

const API: &str = "https://ws.audioscrobbler.com/2.0/";

pub struct LastFm {
    agent: Agent,
    api_key: String,
}

impl LastFm {
    pub fn new(agent: Agent, api_key: impl Into<String>) -> Self {
        Self { agent, api_key: api_key.into() }
    }

    fn get(&self, params: &[(&str, &str)]) -> Result<Value> {
        let mut req = self
            .agent
            .get(API)
            .query("api_key", &self.api_key)
            .query("format", "json")
            .query("autocorrect", "1");
        for (k, v) in params {
            req = req.query(*k, *v);
        }
        let v: Value = req.call()?.body_mut().read_json()?;
        if let Some(code) = v.get("error") {
            let message = v.get("message").and_then(Value::as_str).unwrap_or("unknown error");
            bail!("Last.fm error {code}: {message}");
        }
        Ok(v)
    }

    /// When a track is too obscure for track-level similarity, fall back to
    /// the top tracks of similar artists.
    fn artist_fallback(&self, seed: &Seed, want: usize) -> Result<Vec<Candidate>> {
        let similar =
            self.get(&[("method", "artist.getsimilar"), ("artist", &seed.artist), ("limit", "8")])?;
        let mut out = Vec::new();
        for (i, artist) in as_list(similar.pointer("/similarartists/artist")).iter().enumerate() {
            let Some(name) = artist.get("name").and_then(Value::as_str) else { continue };
            let artist_match = number(artist.get("match")).unwrap_or(0.5);
            let top = match self.get(&[
                ("method", "artist.gettoptracks"),
                ("artist", name),
                ("limit", "3"),
            ]) {
                Ok(v) => v,
                Err(e) => {
                    log::debug!("Last.fm top tracks for {name}: {e:#}");
                    continue;
                }
            };
            for (j, t) in as_list(top.pointer("/toptracks/track")).iter().enumerate() {
                if let Some(title) = t.get("name").and_then(Value::as_str) {
                    out.push(Candidate {
                        title: title.to_owned(),
                        artist: name.to_owned(),
                        // Artist-level evidence is weaker than track-level.
                        similarity: Some(artist_match * 0.5 / (1.0 + j as f64 + i as f64 * 0.1)),
                        ..Default::default()
                    });
                }
            }
            if out.len() >= want {
                break;
            }
        }
        Ok(out)
    }
}

impl Provider for LastFm {
    fn id(&self) -> SourceId {
        SourceId::LastFm
    }

    fn similar(&self, seed: &Seed, limit: usize) -> Result<Vec<Candidate>> {
        let limit_s = limit.to_string();
        let v = self.get(&[
            ("method", "track.getsimilar"),
            ("artist", &seed.artist),
            ("track", &seed.title),
            ("limit", &limit_s),
        ])?;
        let mut out = parse_similar(&v);
        if out.len() < 10 {
            out.extend(self.artist_fallback(seed, limit.saturating_sub(out.len()))?);
        }
        Ok(out)
    }
}

fn parse_similar(v: &Value) -> Vec<Candidate> {
    as_list(v.pointer("/similartracks/track"))
        .iter()
        .filter_map(|t| {
            Some(Candidate {
                title: t.get("name")?.as_str()?.to_owned(),
                artist: t.pointer("/artist/name")?.as_str()?.to_owned(),
                similarity: number(t.get("match")),
                ..Default::default()
            })
        })
        .collect()
}

/// Last.fm returns a bare object instead of a one-element array.
fn as_list(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        Some(o @ Value::Object(_)) => vec![o.clone()],
        _ => Vec::new(),
    }
}

/// Last.fm sends numbers as numbers or as strings depending on the endpoint.
fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_similar_tracks() {
        let v = json!({"similartracks": {"track": [
            {"name": "Save Your Tears", "match": 1.0, "artist": {"name": "The Weeknd"}},
            {"name": "Levitating", "match": "0.83", "artist": {"name": "Dua Lipa"}},
            {"name": "broken"}
        ]}});
        let c = parse_similar(&v);
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].similarity, Some(0.83));
        assert_eq!(c[1].artist, "Dua Lipa");
    }

    #[test]
    fn single_object_is_a_list() {
        let v = json!({"similartracks": {"track": {"name": "A", "artist": {"name": "B"}}}});
        assert_eq!(parse_similar(&v).len(), 1);
    }
}
