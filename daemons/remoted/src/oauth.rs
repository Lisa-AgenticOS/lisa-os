//! OAuth sign-in for subscription-backed providers — "Sign in with
//! Claude" (Anthropic Pro/Max) and "Sign in with ChatGPT" (OpenAI
//! Plus/Pro, the Codex flow). ADR-0010 §4.
//!
//! Every endpoint, client id, redirect, scope and extra parameter below
//! is a VERIFIED public constant ported from the shipping Construct
//! desktop app (github.com/construct-space/construct-app,
//! `brain/oauth/{anthropic,openai_codex}.go` and
//! `brain/provider/anthropic.go`). These are the public halves of OAuth
//! *public clients*, not secrets and not guessed: per CLAUDE.md rule 8
//! the reference implementation is the pinned source, cited inline.
//!
//! Flow (browser-callback, RFC 7636 PKCE S256):
//!   1. `begin_login` binds a loopback callback server on the provider's
//!      fixed port (127.0.0.1:53692 Claude / 127.0.0.1:1455 ChatGPT),
//!      returns the authorize URL. The *panel* opens the browser — the
//!      broker never launches a browser (egress isolation: the callback
//!      listener is loopback-only).
//!   2. The provider redirects the browser back to the loopback server
//!      with `?code=…&state=…`; we validate `state`, exchange the code
//!      for `{access, refresh, expires}`, persist the refresh, cache the
//!      access, and emit a completion event.
//!   3. `access_token` mints/refreshes access tokens on the data path,
//!      re-persisting the rotated refresh (both providers rotate it).

use crate::secrets::SecretStore;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng as _;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

/// Header Anthropic requires alongside an OAuth bearer token on the
/// Messages API (Construct `brain/provider/anthropic.go`).
pub const ANTHROPIC_OAUTH_BETA: &str = "oauth-2025-04-20";

/// The system-prompt marker Anthropic checks for on OAuth (Pro/Max)
/// traffic. Without it the Messages API returns 429 even on the first
/// request — the server treats the call as non-Claude-Code traffic and
/// rejects it. Verbatim from Construct `brain/provider/anthropic.go` and
/// the Claude Code CLI. Prepended to the `system` string when a request
/// authenticates with a Claude subscription token.
pub const ANTHROPIC_CLAUDE_CODE_SYSTEM: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

/// Proactively refresh once the cached access token is within this of
/// its stated expiry.
const EXPIRY_MARGIN_MS: i64 = 60_000;
/// A browser round-trip may involve a login + consent screen; give up on
/// the callback after this and report the login failed.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum OauthError {
    #[error("provider {0:?} does not support OAuth sign-in")]
    NotCapable(String),
    #[error("sign in with {0} again — the stored session is no longer valid")]
    ReauthRequired(String),
    #[error("a sign-in for {0} is already in progress")]
    InProgress(String),
    #[error("could not bind the loopback callback port {0}: {1}")]
    Bind(u16, String),
    #[error("token endpoint returned HTTP {status}: {body}")]
    Token { status: u16, body: String },
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Secrets(#[from] crate::secrets::SecretsError),
    #[error("oauth callback: {0}")]
    Callback(String),
}

/// The exchange/refresh POST body encoding. Anthropic's token endpoint
/// takes JSON; OpenAI's takes form-urlencoded (Construct ports both
/// verbatim from the respective vendor flows).
#[derive(Clone, Copy)]
enum Encoding {
    Json,
    Form,
}

/// One OAuth-capable provider's verified public constants + the small
/// per-vendor flow differences.
pub struct OauthProvider {
    pub id: &'static str,
    /// Human label for the callback page ("Claude" / "ChatGPT").
    pub label: &'static str,
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    pub client_id: &'static str,
    pub redirect_uri: &'static str,
    pub callback_addr: &'static str,
    pub callback_path: &'static str,
    pub callback_port: u16,
    pub scope: &'static str,
    encoding: Encoding,
    /// Extra authorize-URL query params beyond the PKCE/OAuth standard set.
    authorize_extra: &'static [(&'static str, &'static str)],
    /// Anthropic uses the PKCE verifier itself as the OAuth `state`;
    /// OpenAI uses a fresh random state.
    state_is_verifier: bool,
    /// Anthropic echoes `state` in the code-exchange body.
    exchange_sends_state: bool,
    /// OpenAI repeats `scope` on the refresh request.
    refresh_sends_scope: bool,
    /// True for Anthropic: chat requests carry the OAuth bearer + the
    /// `anthropic-beta` header + Claude-Code system prompt + `?beta=true`.
    /// False for OpenAI: a plain `Authorization: Bearer`.
    pub anthropic_bearer: bool,
}

/// Anthropic "Sign in with Claude" (Claude Pro/Max). Constants verified
/// against Construct `brain/oauth/anthropic.go`.
pub const ANTHROPIC: OauthProvider = OauthProvider {
    id: "anthropic",
    label: "Claude",
    authorize_url: "https://claude.ai/oauth/authorize",
    token_url: "https://platform.claude.com/v1/oauth/token",
    client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
    redirect_uri: "http://localhost:53692/callback",
    callback_addr: "127.0.0.1:53692",
    callback_path: "/callback",
    callback_port: 53692,
    scope: "user:profile user:inference",
    encoding: Encoding::Json,
    authorize_extra: &[("code", "true")],
    state_is_verifier: true,
    exchange_sends_state: true,
    refresh_sends_scope: false,
    anthropic_bearer: true,
};

/// OpenAI "Sign in with ChatGPT" (Plus/Pro, the Codex flow). Constants
/// verified against Construct `brain/oauth/openai_codex.go`; `originator`
/// is Lisa's own (Construct sends "construct").
pub const OPENAI: OauthProvider = OauthProvider {
    id: "openai",
    label: "ChatGPT",
    authorize_url: "https://auth.openai.com/oauth/authorize",
    token_url: "https://auth.openai.com/oauth/token",
    client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
    redirect_uri: "http://localhost:1455/auth/callback",
    callback_addr: "127.0.0.1:1455",
    callback_path: "/auth/callback",
    callback_port: 1455,
    scope: "openid profile email offline_access",
    encoding: Encoding::Form,
    authorize_extra: &[
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "lisa"),
    ],
    state_is_verifier: false,
    exchange_sends_state: false,
    refresh_sends_scope: true,
    anthropic_bearer: false,
};

/// The OAuth-capable provider for `id`, if any. Only `anthropic` and
/// `openai` support OAuth today; everything else is key-only.
pub fn provider(id: &str) -> Option<&'static OauthProvider> {
    match id {
        "anthropic" => Some(&ANTHROPIC),
        "openai" => Some(&OPENAI),
        _ => None,
    }
}

/// Whether a provider id can offer "Sign in with …".
pub fn is_capable(id: &str) -> bool {
    provider(id).is_some()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// RFC 7636 PKCE pair (S256).
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// Verifier = BASE64URL-NOPAD(32 random bytes) — 43 chars, matching
    /// Construct's `generatePKCE`.
    pub fn generate() -> Self {
        let mut buf = [0u8; 32];
        rand::rng().fill(&mut buf);
        Self::from_verifier(&URL_SAFE_NO_PAD.encode(buf))
    }

    /// challenge = BASE64URL-NOPAD(SHA256(verifier)) — the S256 method.
    pub fn from_verifier(verifier: &str) -> Self {
        let digest = Sha256::digest(verifier.as_bytes());
        Self {
            verifier: verifier.to_string(),
            challenge: URL_SAFE_NO_PAD.encode(digest),
        }
    }
}

/// A random 16-byte hex `state` (OpenAI's `makeOpenAIState`).
fn random_state() -> String {
    let mut buf = [0u8; 16];
    rand::rng().fill(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the authorize URL for `p` with the given PKCE challenge and
/// `state`. Pure — unit-tested against the verified constants.
pub fn authorize_url(p: &OauthProvider, challenge: &str, state: &str) -> String {
    let mut params: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", p.client_id),
        ("redirect_uri", p.redirect_uri),
        ("scope", p.scope),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ];
    params.extend_from_slice(p.authorize_extra);
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{}", p.authorize_url, query)
}

/// The code→token exchange body (`authorization_code` grant). Pure.
fn exchange_params(p: &OauthProvider, code: &str, verifier: &str) -> Vec<(&'static str, String)> {
    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("client_id", p.client_id.to_string()),
        ("code", code.to_string()),
        ("code_verifier", verifier.to_string()),
        ("redirect_uri", p.redirect_uri.to_string()),
    ];
    if p.exchange_sends_state {
        params.push(("state", verifier.to_string()));
    }
    params
}

/// The refresh body (`refresh_token` grant). Pure.
fn refresh_params(p: &OauthProvider, refresh: &str) -> Vec<(&'static str, String)> {
    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("client_id", p.client_id.to_string()),
        ("refresh_token", refresh.to_string()),
    ];
    if p.refresh_sends_scope {
        params.push(("scope", p.scope.to_string()));
    }
    params
}

/// Parsed token endpoint response.
struct TokenResponse {
    access: String,
    /// Present when the endpoint issued (or rotated) a refresh token.
    refresh: Option<String>,
    expires_ms: i64,
}

/// POST a token request in the provider's encoding and parse the reply.
async fn post_token(
    http: &reqwest::Client,
    p: &OauthProvider,
    params: Vec<(&'static str, String)>,
) -> Result<TokenResponse, OauthError> {
    let req = http.post(p.token_url);
    let req = match p.encoding {
        Encoding::Json => {
            let map: serde_json::Map<String, Value> = params
                .into_iter()
                .map(|(k, v)| (k.to_string(), Value::String(v)))
                .collect();
            req.json(&map)
        }
        Encoding::Form => req.form(&params),
    };
    let resp = req.send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(OauthError::Token {
            status: status.as_u16(),
            body,
        });
    }
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| OauthError::Callback(format!("parse token: {e}")))?;
    let access = v["access_token"].as_str().unwrap_or_default().to_string();
    if access.is_empty() {
        return Err(OauthError::Token {
            status: status.as_u16(),
            body: "token response missing access_token".into(),
        });
    }
    let refresh = v["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);
    let expires_in = v["expires_in"].as_i64().unwrap_or(3600);
    Ok(TokenResponse {
        access,
        refresh,
        expires_ms: now_ms() + expires_in * 1000,
    })
}

/// A 400 whose body carries the OAuth `invalid_grant` code — a
/// permanently-dead refresh token. Anthropic and OpenAI both use the
/// standard OAuth error shape (Construct `isInvalidGrant`).
fn is_invalid_grant(e: &OauthError) -> bool {
    matches!(e, OauthError::Token { status: 400, body } if body.contains("invalid_grant"))
}

/// The persisted OAuth credential: `{ "type": "oauth", "refresh": … }`.
/// The refresh token is the only thing on disk — access tokens live in
/// memory and are re-minted from the refresh (ADR-0010 §3). Written
/// through the 0600 secret store; the refresh is never returned by any
/// read-back surface (write-only, like API keys).
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredOauth {
    #[serde(rename = "type")]
    kind: String,
    refresh: String,
}

/// An access token cached in memory until near its expiry.
struct CachedAccess {
    access: String,
    expires_ms: i64,
}

/// A finished login, broadcast so the D-Bus layer can emit
/// `LoginCompleted`.
#[derive(Clone, Debug)]
pub struct LoginOutcome {
    pub provider_id: String,
    pub ok: bool,
    pub detail: String,
}

/// Owns the OAuth state: verified provider table, in-memory access-token
/// cache, refresh persistence via the secret store, and the login-event
/// broadcast. Held behind an `Arc` so `begin_login` can hand a clone to
/// the spawned callback task.
pub struct OauthManager {
    http: reqwest::Client,
    secrets: SecretStore,
    cache: Mutex<HashMap<String, CachedAccess>>,
    active: Mutex<HashSet<String>>,
    events: broadcast::Sender<LoginOutcome>,
}

fn store_name(id: &str) -> String {
    format!("{id}.oauth.json")
}

impl OauthManager {
    pub fn new(http: reqwest::Client, secrets: SecretStore) -> Arc<Self> {
        let (events, _) = broadcast::channel(16);
        Arc::new(Self {
            http,
            secrets,
            cache: Mutex::new(HashMap::new()),
            active: Mutex::new(HashSet::new()),
            events,
        })
    }

    /// Subscribe to login completions (the D-Bus layer forwards these as
    /// the `LoginCompleted` signal).
    pub fn subscribe(&self) -> broadcast::Receiver<LoginOutcome> {
        self.events.subscribe()
    }

    /// A usable OAuth session (a non-empty refresh) is stored for `id`.
    pub fn connected(&self, id: &str) -> bool {
        self.load_refresh(id).is_some()
    }

    fn load_refresh(&self, id: &str) -> Option<String> {
        let raw = self.secrets.get_named(&store_name(id)).ok()?;
        let stored: StoredOauth = serde_json::from_str(&raw).ok()?;
        (!stored.refresh.is_empty()).then_some(stored.refresh)
    }

    fn persist_refresh(&self, id: &str, refresh: &str) -> Result<(), OauthError> {
        let stored = StoredOauth {
            kind: "oauth".to_string(),
            refresh: refresh.to_string(),
        };
        self.secrets
            .set_named(&store_name(id), &serde_json::to_string(&stored).unwrap())?;
        Ok(())
    }

    /// Forget the stored session (Logout). Idempotent.
    pub fn logout(&self, id: &str) -> Result<(), OauthError> {
        self.secrets.remove_named(&store_name(id))?;
        self.cache.lock().expect("oauth cache").remove(id);
        Ok(())
    }

    /// Start a login: bind the loopback callback server, spawn the
    /// wait→exchange→persist task, and return the authorize URL for the
    /// panel to open. The broker never opens the browser itself.
    pub async fn begin_login(self: &Arc<Self>, id: &str) -> Result<String, OauthError> {
        let p = provider(id).ok_or_else(|| OauthError::NotCapable(id.to_string()))?;
        {
            let mut active = self.active.lock().expect("oauth active");
            if !active.insert(id.to_string()) {
                return Err(OauthError::InProgress(p.label.to_string()));
            }
        }
        let listener = match TcpListener::bind(p.callback_addr).await {
            Ok(l) => l,
            Err(e) => {
                self.active.lock().expect("oauth active").remove(id);
                return Err(OauthError::Bind(p.callback_port, e.to_string()));
            }
        };
        let pkce = Pkce::generate();
        let state = if p.state_is_verifier {
            pkce.verifier.clone()
        } else {
            random_state()
        };
        let url = authorize_url(p, &pkce.challenge, &state);

        let this = Arc::clone(self);
        let verifier = pkce.verifier.clone();
        let id_owned = id.to_string();
        tokio::spawn(async move {
            let outcome = this.run_login(p, listener, &verifier, &state).await;
            this.active.lock().expect("oauth active").remove(&id_owned);
            let _ = this.events.send(outcome);
        });
        Ok(url)
    }

    /// Wait for the callback, exchange the code, persist. Always yields a
    /// `LoginOutcome` (never panics the task).
    async fn run_login(
        &self,
        p: &OauthProvider,
        listener: TcpListener,
        verifier: &str,
        state: &str,
    ) -> LoginOutcome {
        let result =
            match tokio::time::timeout(LOGIN_TIMEOUT, accept_code(&listener, p, state)).await {
                Err(_) => Err(OauthError::Callback(
                    "timed out waiting for the browser".into(),
                )),
                Ok(Err(e)) => Err(e),
                Ok(Ok(code)) => self.complete_login(p, &code, verifier).await,
            };
        match result {
            Ok(()) => LoginOutcome {
                provider_id: p.id.to_string(),
                ok: true,
                detail: format!("{} connected", p.label),
            },
            Err(e) => LoginOutcome {
                provider_id: p.id.to_string(),
                ok: false,
                detail: e.to_string(),
            },
        }
    }

    async fn complete_login(
        &self,
        p: &OauthProvider,
        code: &str,
        verifier: &str,
    ) -> Result<(), OauthError> {
        let resp = post_token(&self.http, p, exchange_params(p, code, verifier)).await?;
        let refresh = resp
            .refresh
            .ok_or_else(|| OauthError::Callback("token response missing refresh_token".into()))?;
        self.persist_refresh(p.id, &refresh)?;
        self.cache.lock().expect("oauth cache").insert(
            p.id.to_string(),
            CachedAccess {
                access: resp.access,
                expires_ms: resp.expires_ms,
            },
        );
        Ok(())
    }

    /// A valid access token for `id`, refreshing (and re-persisting the
    /// rotated refresh) when the cached one is missing or near expiry.
    /// Errors `ReauthRequired` when no refresh is stored or the stored
    /// refresh is permanently invalid — the panel prompts a re-login.
    pub async fn access_token(&self, id: &str) -> Result<String, OauthError> {
        let p = provider(id).ok_or_else(|| OauthError::NotCapable(id.to_string()))?;
        {
            let cache = self.cache.lock().expect("oauth cache");
            if let Some(c) = cache.get(id)
                && c.expires_ms - now_ms() > EXPIRY_MARGIN_MS
            {
                return Ok(c.access.clone());
            }
        }
        let refresh = self
            .load_refresh(id)
            .ok_or_else(|| OauthError::ReauthRequired(p.label.to_string()))?;
        let resp = match post_token(&self.http, p, refresh_params(p, &refresh)).await {
            Ok(r) => r,
            Err(e) if is_invalid_grant(&e) => {
                return Err(OauthError::ReauthRequired(p.label.to_string()));
            }
            Err(e) => return Err(e),
        };
        // Both providers rotate the refresh on every refresh — re-persist
        // the new one BEFORE trusting the new access token, so a write
        // failure can't strand a rotated refresh only in memory.
        if let Some(new_refresh) = &resp.refresh
            && new_refresh != &refresh
        {
            self.persist_refresh(id, new_refresh)?;
        }
        self.cache.lock().expect("oauth cache").insert(
            id.to_string(),
            CachedAccess {
                access: resp.access.clone(),
                expires_ms: resp.expires_ms,
            },
        );
        Ok(resp.access)
    }
}

// ---- loopback callback server --------------------------------------------

/// Accept connections until the provider's callback path delivers a
/// `code` (with a matching `state`), or the browser reports an error.
/// Other paths (favicon, etc.) get a 404 and the loop continues.
async fn accept_code(
    listener: &TcpListener,
    p: &OauthProvider,
    expected_state: &str,
) -> Result<String, OauthError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| OauthError::Callback(e.to_string()))?;
        let Some(target) = read_request_target(&mut stream).await else {
            write_response(&mut stream, 400, &error_html(p.label, "Malformed request.")).await;
            continue;
        };
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        if path != p.callback_path {
            write_response(&mut stream, 404, "not found").await;
            continue;
        }
        let params = parse_query(query);
        if let Some(err) = params.get("error") {
            write_response(&mut stream, 400, &error_html(p.label, err)).await;
            return Err(OauthError::Callback(err.clone()));
        }
        let state = params.get("state").map(String::as_str).unwrap_or_default();
        let code = params.get("code").map(String::as_str).unwrap_or_default();
        if state != expected_state {
            let msg = "State mismatch — close other sign-in tabs and retry.";
            write_response(&mut stream, 400, &error_html(p.label, msg)).await;
            return Err(OauthError::Callback("state mismatch".into()));
        }
        if code.is_empty() {
            let msg = "Missing authorization code — consent was probably cancelled.";
            write_response(&mut stream, 400, &error_html(p.label, msg)).await;
            return Err(OauthError::Callback("missing authorization code".into()));
        }
        write_response(&mut stream, 200, &success_html(p.label)).await;
        return Ok(code.to_string());
    }
}

/// Read the HTTP request line's target (`GET <target> HTTP/1.1`). A
/// browser redirect delivers the whole request line in the first packet.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).await.ok()?;
    let text = String::from_utf8_lossy(&buf[..n]);
    let line = text.lines().next()?;
    line.split_whitespace().nth(1).map(str::to_string)
}

async fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.flush().await;
}

fn parse_query(q: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(k), percent_decode(v));
    }
    map
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 3 <= bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A minimal, self-contained success page (no external assets) — the
/// spirit of Construct's `callback_html.go`: warm canvas, oversized
/// headline ending in an accent dot (Lisa's egress color).
fn success_html(label: &str) -> String {
    page(
        &format!("{label} connected"),
        "You're in.",
        &format!(
            "Your {} account is linked to Lisa. You can close this tab.",
            escape(label)
        ),
        false,
    )
}

fn error_html(label: &str, reason: &str) -> String {
    page(
        "Sign-in failed",
        "Sign-in failed.",
        &format!(
            "Couldn't link {} to Lisa: {} Close this tab and try again from Settings.",
            escape(label),
            escape(reason)
        ),
        true,
    )
}

fn page(eyebrow: &str, headline: &str, body: &str, error: bool) -> String {
    let accent = if error { "#D7263D" } else { "#E66100" };
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>Lisa — {eyebrow}</title><style>\
:root{{color-scheme:light dark}}\
*{{box-sizing:border-box}}html,body{{height:100%;margin:0}}\
body{{font:15px/1.55 -apple-system,BlinkMacSystemFont,system-ui,sans-serif;\
background:#FBF7F5;color:#14110F;display:flex;align-items:center;justify-content:center;padding:48px}}\
@media(prefers-color-scheme:dark){{body{{background:#14110F;color:#FBF7F5}}}}\
.hero{{max-width:560px}}\
.eyebrow{{font-size:12px;font-weight:700;letter-spacing:.22em;text-transform:uppercase;color:{accent};margin-bottom:16px}}\
h1{{font-size:56px;font-weight:800;letter-spacing:-.02em;line-height:1;margin:0 0 20px}}\
h1 .dot{{color:{accent}}}\
p{{font-size:16px;opacity:.75;margin:0}}\
</style></head><body><div class=\"hero\">\
<div class=\"eyebrow\">{eyebrow}</div>\
<h1>{headline}<span class=\"dot\">.</span></h1><p>{body}</p></div></body></html>",
        eyebrow = escape(eyebrow),
        headline = escape(headline),
        body = body,
        accent = accent,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s256_matches_the_rfc_7636_appendix_b_vector() {
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_verifiers_are_43_char_base64url_and_distinct() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_eq!(a.verifier.len(), 43, "32 bytes base64url-nopad");
        assert_ne!(a.verifier, b.verifier);
        assert!(
            a.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        );
    }

    #[test]
    fn only_anthropic_and_openai_are_oauth_capable() {
        assert!(is_capable("anthropic"));
        assert!(is_capable("openai"));
        for id in ["tinker", "together", "google", "openrouter", "homelab"] {
            assert!(!is_capable(id), "{id} must stay key-only");
        }
    }

    #[test]
    fn anthropic_authorize_url_carries_the_verified_constants() {
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        let url = authorize_url(&ANTHROPIC, &pkce.challenge, &pkce.verifier);
        assert!(url.starts_with("https://claude.ai/oauth/authorize?"));
        assert!(url.contains("client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
        // scope + redirect are percent-encoded.
        assert!(url.contains("scope=user%3Aprofile%20user%3Ainference"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A53692%2Fcallback"));
        // Anthropic quirks: code=true, state == the PKCE verifier.
        assert!(url.contains("code=true"));
        assert!(url.contains(&format!("state={}", pkce.verifier)));
    }

    #[test]
    fn openai_authorize_url_carries_the_verified_constants_and_extras() {
        let pkce = Pkce::generate();
        let state = "deadbeef";
        let url = authorize_url(&OPENAI, &pkce.challenge, state);
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(url.contains("scope=openid%20profile%20email%20offline_access"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
        assert!(url.contains("id_token_add_organizations=true"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("originator=lisa"));
        assert!(url.contains("state=deadbeef"));
    }

    #[test]
    fn exchange_and_refresh_bodies_match_each_vendor_flow() {
        // Anthropic: echoes state == verifier, no scope on refresh.
        let ax = exchange_params(&ANTHROPIC, "the-code", "the-verifier");
        assert!(ax.contains(&("grant_type", "authorization_code".into())));
        assert!(ax.contains(&("code_verifier", "the-verifier".into())));
        assert!(ax.contains(&("state", "the-verifier".into())));
        let ar = refresh_params(&ANTHROPIC, "r1");
        assert!(ar.contains(&("grant_type", "refresh_token".into())));
        assert!(ar.contains(&("refresh_token", "r1".into())));
        assert!(!ar.iter().any(|(k, _)| *k == "scope"));

        // OpenAI: no state in exchange, repeats scope on refresh.
        let ox = exchange_params(&OPENAI, "c", "v");
        assert!(!ox.iter().any(|(k, _)| *k == "state"));
        let or = refresh_params(&OPENAI, "r2");
        assert!(or.contains(&("scope", "openid profile email offline_access".into())));
    }

    #[test]
    fn invalid_grant_is_recognized() {
        let e = OauthError::Token {
            status: 400,
            body: r#"{"error":"invalid_grant"}"#.into(),
        };
        assert!(is_invalid_grant(&e));
        let other = OauthError::Token {
            status: 401,
            body: "invalid_grant".into(),
        };
        assert!(!is_invalid_grant(&other));
    }

    #[test]
    fn percent_decoding_handles_codes_and_plus() {
        assert_eq!(percent_decode("a%2Fb%3Dc"), "a/b=c");
        assert_eq!(percent_decode("x+y"), "x y");
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn refresh_persistence_rotates_and_logout_clears() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = SecretStore::open(dir.path()).unwrap();
        let mgr = OauthManager::new(reqwest::Client::new(), secrets.clone());

        assert!(!mgr.connected("anthropic"));
        mgr.persist_refresh("anthropic", "refresh-1").unwrap();
        assert!(mgr.connected("anthropic"));
        assert_eq!(mgr.load_refresh("anthropic").as_deref(), Some("refresh-1"));
        // Stored as the tagged oauth credential; the refresh is not a key file.
        let raw = secrets.get_named("anthropic.oauth.json").unwrap();
        assert!(raw.contains("\"type\":\"oauth\""));

        // Rotation: a new refresh replaces the old on disk.
        mgr.persist_refresh("anthropic", "refresh-2").unwrap();
        assert_eq!(mgr.load_refresh("anthropic").as_deref(), Some("refresh-2"));

        mgr.logout("anthropic").unwrap();
        assert!(!mgr.connected("anthropic"));
    }
}
