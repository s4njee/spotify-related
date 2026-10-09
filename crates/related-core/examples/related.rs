//! Print the fused related-track list for a song, without Spotify.
//!
//! ```sh
//! cargo run -p related-core --example related -- "The Weeknd" "Blinding Lights" [ISRC]
//! LASTFM_API_KEY=... cargo run -p related-core --example related -- "Kate Bush" "Running Up That Hill"
//! ```

use related_core::related::{Deezer, FusionOptions, LastFm, ListenBrainz, Provider, Seed, fuse};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(artist), Some(title)) = (args.next(), args.next()) else {
        anyhow::bail!("usage: related <artist> <title> [isrc]");
    };
    let seed = Seed { title, artist, isrc: args.next(), spotify_id: None };
    let agent = related_core::http::agent();
    let mut providers: Vec<Box<dyn Provider>> =
        vec![Box::new(ListenBrainz::new(agent.clone())), Box::new(Deezer::new(agent.clone()))];
    if let Ok(key) = std::env::var("LASTFM_API_KEY") {
        providers.insert(0, Box::new(LastFm::new(agent, key)));
    }
    let mut lists = Vec::new();
    for p in &providers {
        match p.similar(&seed, 60) {
            Ok(list) => {
                println!("{:<13} {} suggestions", p.id().label(), list.len());
                lists.push((p.id(), list));
            }
            Err(e) => println!("{:<13} failed: {e:#}", p.id().label()),
        }
    }
    println!();
    for (i, t) in fuse(&seed, &lists, &FusionOptions::default()).iter().enumerate() {
        let sources: Vec<_> =
            t.hits.iter().map(|h| format!("{}#{}", h.source.label(), h.rank + 1)).collect();
        println!(
            "{:>2}. {:.2}  {} - {}   [{}]",
            i + 1,
            t.score,
            t.artist,
            t.title,
            sources.join(", ")
        );
    }
    Ok(())
}
