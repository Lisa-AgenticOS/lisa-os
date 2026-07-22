//! "Sign in with Claude" OAuth machinery (ADR-0008 §4).
//!
//! What is implemented: RFC 7636 PKCE (S256), authorize-URL
//! construction, the form-encoded authorization-code exchange request,
//! and the bearer headers Anthropic documents for OAuth tokens
//! (`Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20`).
//!
//! What is deliberately NOT here: the authorize/token endpoint URLs and
//! a client_id. Anthropic publishes no registerable third-party OAuth
//! client today (their docs list API keys and Workload Identity
//! Federation for third-party software), so per CLAUDE.md rule 8 those
//! values ship **explicitly unset** in `oauth.toml` and the flow reports
//! `Unconfigured` until real values exist. No invented URLs.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Header value Anthropic documents as required alongside OAuth bearer
/// tokens on the Messages API.
pub const ANTHROPIC_OAUTH_BETA: &str = "oauth-2025-04-20";

#[derive(Debug, thiserror::Error)]
pub enum OauthError {
    #[error(
        "Sign in with Claude is not configured: Anthropic publishes no registerable \
         third-party OAuth client today; authorize_url/token_url/client_id in oauth.toml \
         are unset (ADR-0008 §4)"
    )]
    Unconfigured,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("oauth.toml: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Endpoint configuration. Every field defaults to None — set them in
/// `<state>/oauth.toml` when Anthropic publishes a client program.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OauthEndpoints {
    pub authorize_url: Option<String>,
    pub token_url: Option<String>,
    pub client_id: Option<String>,
    /// e.g. an out-of-band/loopback redirect once documented.
    pub redirect_uri: Option<String>,
    /// Space-separated scope string, if the program defines one.
    pub scope: Option<String>,
}

impl OauthEndpoints {
    pub fn load(state_dir: &Path) -> Result<Self, OauthError> {
        let path = state_dir.join("oauth.toml");
        match std::fs::read_to_string(&path) {
            Ok(raw) => Ok(toml::from_str(&raw)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn configured(&self) -> bool {
        self.authorize_url.is_some() && self.token_url.is_some() && self.client_id.is_some()
    }
}

/// RFC 7636 PKCE pair.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// 64-char verifier from the RFC 7636 unreserved charset.
    pub fn generate() -> Self {
        use rand::Rng;
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
        let mut rng = rand::rng();
        let verifier: String = (0..64)
            .map(|_| CHARS[rng.random_range(0..CHARS.len())] as char)
            .collect();
        Self::from_verifier(&verifier)
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

/// A started sign-in: hand `authorize_url` to the browser, keep
/// `verifier` + `state` for the exchange.
#[derive(Debug, Clone, Serialize)]
pub struct AuthorizeRequest {
    pub authorize_url: String,
    pub state: String,
    pub verifier: String,
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

/// Build the authorization request (code + PKCE S256). Errors with
/// `Unconfigured` until real endpoints are supplied.
pub fn authorize_request(
    endpoints: &OauthEndpoints,
    pkce: &Pkce,
) -> Result<AuthorizeRequest, OauthError> {
    let (Some(base), Some(client_id)) = (&endpoints.authorize_url, &endpoints.client_id) else {
        return Err(OauthError::Unconfigured);
    };
    if endpoints.token_url.is_none() {
        return Err(OauthError::Unconfigured);
    }
    let state = Pkce::generate().verifier[..32].to_string();
    let mut url = format!(
        "{base}?response_type=code&client_id={}&code_challenge={}&code_challenge_method=S256&state={}",
        urlencode(client_id),
        urlencode(&pkce.challenge),
        urlencode(&state),
    );
    if let Some(r) = &endpoints.redirect_uri {
        url.push_str(&format!("&redirect_uri={}", urlencode(r)));
    }
    if let Some(s) = &endpoints.scope {
        url.push_str(&format!("&scope={}", urlencode(s)));
    }
    Ok(AuthorizeRequest {
        authorize_url: url,
        state,
        verifier: pkce.verifier.clone(),
    })
}

/// The form body for the code → token exchange (posted
/// `application/x-www-form-urlencoded` to `token_url`).
pub fn token_exchange_form(
    endpoints: &OauthEndpoints,
    code: &str,
    verifier: &str,
) -> Result<Vec<(String, String)>, OauthError> {
    let Some(client_id) = &endpoints.client_id else {
        return Err(OauthError::Unconfigured);
    };
    if endpoints.token_url.is_none() {
        return Err(OauthError::Unconfigured);
    }
    let mut form = vec![
        ("grant_type".to_string(), "authorization_code".to_string()),
        ("code".to_string(), code.to_string()),
        ("client_id".to_string(), client_id.clone()),
        ("code_verifier".to_string(), verifier.to_string()),
    ];
    if let Some(r) = &endpoints.redirect_uri {
        form.push(("redirect_uri".to_string(), r.clone()));
    }
    Ok(form)
}

/// Headers for authenticating an Anthropic request with an OAuth token.
pub fn bearer_headers(token: &str) -> Vec<(String, String)> {
    vec![
        ("authorization".into(), format!("Bearer {token}")),
        ("anthropic-beta".into(), ANTHROPIC_OAUTH_BETA.into()),
    ]
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
    fn generated_verifiers_are_valid_and_distinct() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_eq!(a.verifier.len(), 64);
        assert_ne!(a.verifier, b.verifier);
        assert!(
            a.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c))
        );
    }

    #[test]
    fn defaults_are_unset_and_flow_reports_unconfigured() {
        let dir = tempfile::tempdir().unwrap();
        let e = OauthEndpoints::load(dir.path()).unwrap();
        assert!(!e.configured(), "rule 8: endpoints ship unset");
        assert!(matches!(
            authorize_request(&e, &Pkce::generate()),
            Err(OauthError::Unconfigured)
        ));
        assert!(matches!(
            token_exchange_form(&e, "code", "verifier"),
            Err(OauthError::Unconfigured)
        ));
    }

    #[test]
    fn configured_endpoints_build_a_pkce_authorize_url_and_exchange_form() {
        let e = OauthEndpoints {
            authorize_url: Some("https://example.test/oauth/authorize".into()),
            token_url: Some("https://example.test/oauth/token".into()),
            client_id: Some("client id".into()),
            redirect_uri: Some("http://127.0.0.1:0/cb".into()),
            scope: None,
        };
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        let req = authorize_request(&e, &pkce).unwrap();
        assert!(
            req.authorize_url
                .starts_with("https://example.test/oauth/authorize?")
        );
        assert!(req.authorize_url.contains("code_challenge_method=S256"));
        assert!(
            req.authorize_url
                .contains("code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        );
        assert!(req.authorize_url.contains("client_id=client%20id"));

        let form = token_exchange_form(&e, "auth-code", &req.verifier).unwrap();
        assert!(form.contains(&("grant_type".into(), "authorization_code".into())));
        assert!(form.contains(&("code_verifier".into(), pkce.verifier)));
    }

    #[test]
    fn oauth_bearer_carries_the_documented_beta_header() {
        let h = bearer_headers("tok");
        assert!(h.contains(&("authorization".into(), "Bearer tok".into())));
        assert!(h.contains(&("anthropic-beta".into(), "oauth-2025-04-20".into())));
    }
}
