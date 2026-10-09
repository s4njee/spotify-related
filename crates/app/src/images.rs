//! Album art, fetched on the background executor and kept in memory.
//!
//! GPUI's `img()` can load URLs only when the app installs an HTTP client, so
//! the player downloads the bytes itself with the same agent it uses for the
//! APIs and hands GPUI a decoded-on-demand `Image`.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_pre::{Image, ImageFormat};

pub enum ArtState {
    Loading,
    Ready(Arc<Image>),
    Failed,
}

#[derive(Default)]
pub struct ArtCache {
    entries: HashMap<String, ArtState>,
}

impl ArtCache {
    pub fn get(&self, url: &str) -> Option<&ArtState> {
        self.entries.get(url)
    }

    /// Mark `url` as loading. Returns false if it is already known.
    pub fn begin(&mut self, url: &str) -> bool {
        if self.entries.contains_key(url) {
            return false;
        }
        // Covers are small; a few hundred is plenty for a session.
        if self.entries.len() > 600 {
            self.entries.clear();
        }
        self.entries.insert(url.to_owned(), ArtState::Loading);
        true
    }

    pub fn finish(&mut self, url: String, bytes: Option<Vec<u8>>) {
        let state = bytes
            .and_then(|b| format_of(&b).map(|f| ArtState::Ready(Arc::new(Image::from_bytes(f, b)))))
            .unwrap_or(ArtState::Failed);
        self.entries.insert(url, state);
    }
}

/// Download an image. Runs on a background thread.
pub fn fetch(agent: &related_core::http::Agent, url: &str) -> Option<Vec<u8>> {
    related_core::http::fetch_bytes(agent, url, 4 * 1024 * 1024)
}

fn format_of(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0xFF, 0xD8, ..] => Some(ImageFormat::Jpeg),
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageFormat::Webp),
        [b'G', b'I', b'F', ..] => Some(ImageFormat::Gif),
        _ => None,
    }
}
