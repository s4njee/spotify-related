//! Spotify authorization code flow with PKCE.
//!
//! The app has no client secret: it proves possession of a random verifier
//! instead, which is the flow Spotify recommends for desktop apps. The
//! redirect lands on a one-shot HTTP listener bound to 127.0.0.1.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ureq::Agent;

const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// Only playback scopes: the app never reads or changes your library.
pub const SCOPES: &[&str] =
    &["user-read-playback-state", "user-modify-playback-state", "user-read-currently-playing"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: u64,
}

impl Token {
    pub fn expires_within(&self, seconds: u64) -> bool {
        now() + seconds >= self.expires_at
    }
}

/// One in-flight authorization attempt.
pub struct PkceSession {
    pub client_id: String,
    pub redirect_uri: String,
    verifier: String,
    state: String,
}

impl PkceSession {
    pub fn new(client_id: &str, redirect_uri: &str) -> Self {
        Self {
            client_id: client_id.to_owned(),
            redirect_uri: redirect_uri.to_owned(),
            verifier: random_string(64),
            state: random_string(24),
        }
    }

    pub fn authorize_url(&self) -> String {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()));
        let mut url = url::Url::parse(AUTHORIZE_URL).expect("static URL");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &challenge)
            .append_pair("state", &self.state)
            .append_pair("scope", &SCOPES.join(" "));
        url.into()
    }

    pub fn exchange(&self, agent: &Agent, code: &str) -> Result<Token> {
        let response = agent.post(TOKEN_URL).send_form([
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("client_id", self.client_id.as_str()),
            ("code_verifier", self.verifier.as_str()),
        ])?;
        parse_token_response(response, None)
    }
}

/// Run the whole flow: listen, hand the authorize URL to `open_browser`, wait
/// for the redirect, and exchange the code. `cancel` aborts the wait.
pub fn authorize(
    agent: &Agent,
    client_id: &str,
    port: u16,
    open_browser: impl FnOnce(&str),
    cancel: Arc<AtomicBool>,
) -> Result<Token> {
    let redirect_uri = crate::config::redirect_uri(port);
    let session = PkceSession::new(client_id, &redirect_uri);
    // Bind before opening the browser so a fast redirect cannot miss us.
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("port {port} is busy; set redirect_port in the config"))?;
    open_browser(&session.authorize_url());
    let code = wait_for_code(&listener, &session.state, Duration::from_secs(300), &cancel)?;
    session.exchange(agent, &code)
}

pub fn refresh(agent: &Agent, client_id: &str, refresh_token: &str) -> Result<Token> {
    let response = agent.post(TOKEN_URL).send_form([
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ])?;
    parse_token_response(response, Some(refresh_token))
}

fn parse_token_response(
    mut response: ureq::http::Response<ureq::Body>,
    previous_refresh: Option<&str>,
) -> Result<Token> {
    #[derive(Deserialize)]
    struct Raw {
        access_token: String,
        expires_in: u64,
        refresh_token: Option<String>,
    }
    let status = response.status();
    let body = response.body_mut().read_to_string()?;
    if !status.is_success() {
        let detail: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let message = detail
            .get("error_description")
            .or_else(|| detail.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or(&body);
        bail!("Spotify sign-in failed ({status}): {message}");
    }
    let raw: Raw = serde_json::from_str(&body).context("unexpected token response")?;
    let refresh_token = raw
        .refresh_token
        .or_else(|| previous_refresh.map(str::to_owned))
        .ok_or_else(|| anyhow!("Spotify did not return a refresh token"))?;
    Ok(Token {
        access_token: raw.access_token,
        refresh_token,
        expires_at: now() + raw.expires_in.saturating_sub(30),
    })
}

/// Accept connections until one carries `/callback?code=…&state=…`.
fn wait_for_code(
    listener: &TcpListener,
    expected_state: &str,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<String> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("sign-in cancelled");
        }
        if Instant::now() > deadline {
            bail!("timed out waiting for Spotify to redirect back");
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = handle_callback(stream, expected_state)? {
                    return result;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Returns `None` for requests that are not the callback (such as a favicon).
fn handle_callback(mut stream: TcpStream, expected_state: &str) -> Result<Option<Result<String>>> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
    if url.path() != "/callback" {
        respond(&mut stream, "404 Not Found", "Not found.");
        return Ok(None);
    }
    let get = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
    let outcome = if get("state").as_deref() != Some(expected_state) {
        Err(anyhow!("sign-in response did not match this request"))
    } else if let Some(error) = get("error") {
        Err(anyhow!("Spotify sign-in was not completed: {error}"))
    } else {
        get("code").ok_or_else(|| anyhow!("Spotify redirect had no code"))
    };
    match &outcome {
        Ok(_) => respond(
            &mut stream,
            "200 OK",
            "Signed in to spotify-related. You can close this tab and return to the app.",
        ),
        Err(e) => respond(&mut stream, "400 Bad Request", &e.to_string()),
    }
    Ok(Some(outcome))
}

fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>spotify-related</title>\
         <body style=\"font:16px system-ui;background:#0a0a0a;color:#fafafa;display:grid;\
         place-items:center;height:100vh;margin:0\"><p>{}</p>",
        html_escape(message)
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn random_string(len: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::rng();
    (0..len).map(|_| CHARSET[rng.random_range(0..CHARSET.len())] as char).collect()
}

pub(crate) fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_has_pkce_fields() {
        let session = PkceSession::new("cid", "http://127.0.0.1:8898/callback");
        let url = url::Url::parse(&session.authorize_url()).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "cid");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"].len(), 43);
        assert!(q["scope"].contains("user-modify-playback-state"));
    }

    #[test]
    fn callback_round_trip() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let mut favicon = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(favicon, "GET /favicon.ico HTTP/1.1\r\n\r\n").unwrap();
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET /callback?code=abc&state=xyz HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut out = String::new();
            std::io::Read::read_to_string(&mut s, &mut out).unwrap();
            out
        });
        let cancel = AtomicBool::new(false);
        let code = wait_for_code(&listener, "xyz", Duration::from_secs(5), &cancel).unwrap();
        assert_eq!(code, "abc");
        assert!(client.join().unwrap().starts_with("HTTP/1.1 200 OK"));
    }

    #[test]
    fn callback_rejects_wrong_state() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET /callback?code=abc&state=evil HTTP/1.1\r\n\r\n").unwrap();
        });
        let cancel = AtomicBool::new(false);
        assert!(wait_for_code(&listener, "xyz", Duration::from_secs(5), &cancel).is_err());
    }
}
