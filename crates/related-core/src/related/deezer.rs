//! Deezer artist radio: a mix of the seed artist and similar artists. Coarser
//! than the track-level sources, but it covers new releases and regional
//! catalogs well, and it needs no key.

use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use ureq::Agent;

use super::{Candidate, Provider, Seed, SourceId};
use crate::normalize::artist_key;

const API: &str = "https://api.deezer.com";

pub struct Deezer {
    agent: Agent,
}

impl Deezer {
    pub fn new(agent: Agent) -> Self {
        Self { agent }
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let mut req = self.agent.get(format!("{API}{path}"));
        for (k, v) in query {
            req = req.query(*k, *v);
        }
        let v: Value = req.call()?.body_mut().read_json()?;
        if let Some(e) = v.get("error") {
            bail!("Deezer: {}", e.get("message").and_then(Value::as_str).unwrap_or("error"));
        }
        Ok(v)
    }

    fn artist_id(&self, seed: &Seed) -> Result<u64> {
        if let Some(isrc) = &seed.isrc
            && let Ok(track) = self.get(&format!("/track/isrc:{isrc}"), &[])
            && let Some(id) = track.pointer("/artist/id").and_then(Value::as_u64)
        {
            return Ok(id);
        }
        let q = format!("{} {}", seed.artist, seed.title);
        let v = self.get("/search", &[("q", &q), ("limit", "5")])?;
        let want = artist_key(&seed.artist);
        v.get("data")
            .and_then(Value::as_array)
            .and_then(|hits| {
                hits.iter().find(|h| {
                    h.pointer("/artist/name").and_then(Value::as_str).map(artist_key).as_deref()
                        == Some(want.as_str())
                })
            })
            .and_then(|h| h.pointer("/artist/id").and_then(Value::as_u64))
            .ok_or_else(|| anyhow!("Deezer has no match for {}", seed.artist))
    }
}

impl Provider for Deezer {
    fn id(&self) -> SourceId {
        SourceId::Deezer
    }

    fn similar(&self, seed: &Seed, limit: usize) -> Result<Vec<Candidate>> {
        let artist = self.artist_id(seed)?;
        let limit = limit.to_string();
        let v = self.get(&format!("/artist/{artist}/radio"), &[("limit", &limit)])?;
        Ok(parse_tracks(&v))
    }
}

fn parse_tracks(v: &Value) -> Vec<Candidate> {
    v.get("data")
        .and_then(Value::as_array)
        .map(|tracks| {
            tracks
                .iter()
                .filter_map(|t| {
                    Some(Candidate {
                        title: t.get("title")?.as_str()?.to_owned(),
                        artist: t.pointer("/artist/name")?.as_str()?.to_owned(),
                        album: t.pointer("/album/title").and_then(Value::as_str).map(str::to_owned),
                        isrc: t.get("isrc").and_then(Value::as_str).map(str::to_owned),
                        art_url: t
                            .pointer("/album/cover_medium")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        similarity: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_radio() {
        let v = json!({"data": [
            {"title": "Wanderlust", "artist": {"name": "The Weeknd"},
             "album": {"title": "Kiss Land", "cover_medium": "c"}}
        ]});
        let c = parse_tracks(&v);
        assert_eq!(c[0].album.as_deref(), Some("Kiss Land"));
        assert_eq!(c[0].art_url.as_deref(), Some("c"));
    }
}
