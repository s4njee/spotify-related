//! Everything in spotify-related that is not UI: configuration, the Spotify Web
//! API client, the external related-track providers, and the engine that fuses
//! their answers into one ranked list of playable Spotify tracks.
//!
//! The crate is synchronous on purpose. The GPUI app runs these calls on its
//! background executor, and keeping the core free of an async runtime makes it
//! easy to test and to reuse from a CLI.

pub mod config;
pub mod engine;
pub mod http;
pub mod model;
pub mod normalize;
pub mod related;
pub mod spotify;

pub use config::Config;
pub use engine::{RelatedEngine, RelatedReport, SourceStatus};
pub use model::{Device, PlaybackState, RepeatMode, Track};
pub use related::{FusionOptions, RelatedTrack, Seed, SourceId};
pub use spotify::SpotifyClient;
