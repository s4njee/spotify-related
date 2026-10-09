//! Matching keys for tracks that come from different catalogs.
//!
//! Last.fm, ListenBrainz, Deezer and Spotify spell the same recording in
//! different ways ("Song - 2011 Remaster", "Song (feat. X)", "Beyoncé" versus
//! "Beyonce"). These functions reduce titles and artists to a key that is
//! stable across those variations without merging genuinely different
//! recordings such as remixes and live versions.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Words that mark a parenthetical or dash suffix as packaging rather than a
/// different recording.
const PACKAGING: &[&str] = &[
    "feat", "ft", "featuring", "with", "remaster", "remastered", "mono", "stereo", "deluxe",
    "bonus", "explicit", "clean", "radio edit", "single version", "album version", "original mix",
    "from", "bonus track",
];

/// Lowercase, strip accents, and keep only letters, digits and single spaces.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.nfkd().filter(|c| !is_combining_mark(*c)) {
        let c = match c {
            '&' => ' ',
            '\u{2019}' | '\'' => continue,
            c => c,
        };
        if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.extend(c.to_lowercase());
        } else {
            space = true;
        }
    }
    out
}

fn is_packaging(fragment: &str) -> bool {
    let folded = fold(fragment);
    PACKAGING.iter().any(|word| {
        folded == *word
            || folded.starts_with(&format!("{word} "))
            || folded.ends_with(&format!(" {word}"))
            || folded.contains(&format!(" {word} "))
    }) || folded.chars().all(|c| c.is_ascii_digit() || c == ' ')
}

/// Normalize a track title for matching.
pub fn title_key(title: &str) -> String {
    let mut t = title.to_owned();
    // "Song - 2011 Remaster", "Song - Radio Edit"
    if let Some((head, tail)) = t.split_once(" - ") {
        if is_packaging(tail) {
            t = head.to_owned();
        }
    }
    // "(feat. X)", "[Remastered]"
    let mut out = String::with_capacity(t.len());
    let mut rest = t.as_str();
    while let Some(open) = rest.find(['(', '[']) {
        let close_char = if rest.as_bytes()[open] == b'(' { ')' } else { ']' };
        let Some(close) = rest[open..].find(close_char).map(|c| open + c) else { break };
        out.push_str(&rest[..open]);
        let inner = &rest[open + 1..close];
        if !is_packaging(inner) {
            out.push(' ');
            out.push_str(inner);
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    fold(&out)
}

/// Normalize an artist credit for matching: keep the first credited artist and
/// drop a leading "the". Catalogs that credit "A & B" as one string then match
/// Spotify, which lists A and B separately. A duo such as "Simon & Garfunkel"
/// reduces to "simon" on every side, so it still matches itself.
pub fn artist_key(artist: &str) -> String {
    let lower = artist.to_lowercase();
    let mut cut = lower.len();
    for sep in [" feat. ", " feat ", " ft. ", " ft ", " featuring ", ", ", " & ", " x ", " vs. ", " with "] {
        if let Some(i) = lower.find(sep) {
            cut = cut.min(i);
        }
    }
    let folded = fold(&lower[..cut]);
    folded.strip_prefix("the ").map(str::to_owned).unwrap_or(folded)
}

/// Key identifying one recording across catalogs.
pub fn track_key(artist: &str, title: &str) -> String {
    format!("{}\u{1f}{}", artist_key(artist), title_key(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_packaging_but_not_versions() {
        assert_eq!(title_key("Heroes - 2017 Remaster"), "heroes");
        assert_eq!(title_key("Bad Habits (feat. Bruno Mars)"), "bad habits");
        assert_eq!(title_key("Something [Remastered 2009]"), "something");
        assert_eq!(title_key("One More Time (Radio Edit)"), "one more time");
        assert_ne!(title_key("Blinding Lights (Chromatics Remix)"), title_key("Blinding Lights"));
        assert_ne!(title_key("Creep - Live"), title_key("Creep"));
    }

    #[test]
    fn folds_accents_and_punctuation() {
        assert_eq!(fold("Beyoncé"), "beyonce");
        assert_eq!(fold("Don’t Start Now"), "dont start now");
        assert_eq!(fold("  Mr.   Brightside!! "), "mr brightside");
    }

    #[test]
    fn artist_keys_match_across_catalogs() {
        assert_eq!(artist_key("The Weeknd"), "weeknd");
        assert_eq!(artist_key("Weeknd, The"), "weeknd");
        assert_eq!(artist_key("Calvin Harris feat. Rihanna"), "calvin harris");
        assert_eq!(artist_key("Post Malone & Swae Lee"), artist_key("Post Malone"));
        assert_eq!(artist_key("Simon & Garfunkel"), "simon");
        assert_eq!(
            track_key("Dua Lipa", "Don't Start Now"),
            track_key("DUA LIPA", "Don’t Start Now")
        );
    }
}
