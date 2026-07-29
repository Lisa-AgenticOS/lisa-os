//! Where the one daemon with egress is allowed to dial (issues #92, #96,
//! #109).
//!
//! `lisa-remoted` is the only Lisa process on the network (CLAUDE.md
//! rule 5). That makes it the one place where "a string the user typed"
//! turns into "a connection from inside this machine's network
//! position", and the check used to be:
//!
//! ```ignore
//! if !(base_url.starts_with("https://") || base_url.starts_with("http://"))
//! ```
//!
//! No parse, no host, no port, no userinfo check. So
//! `http://169.254.169.254/latest/meta-data` and `http://10.0.0.1/` were
//! valid providers, and the broker really dialled them, forwarding the
//! stored credential. A general-purpose HTTP client pointed *inward* is
//! the opposite of what an egress broker is for.
//!
//! # Two checks, because one is not enough
//!
//! **At registration** ([`validate_base_url`]) the URL must parse, carry
//! a host, use `https`, and carry no userinfo. A literal address must be
//! a public one.
//!
//! **At connect time** ([`GuardedResolver`]) every resolved address is
//! checked again. A registration-time check alone is decoration against
//! DNS: `evil.example` resolves publicly when you add it and to
//! `127.0.0.1` when the request goes out. The name is looked up by the
//! same resolver that will be used to dial, and addresses that are not
//! public are dropped before a socket is opened.
//!
//! # The escape hatch, per provider and not per daemon
//!
//! Self-hosted models on `localhost:11434` or a box on the LAN are a
//! real and ordinary thing to want. That is allowed — but as a property
//! of *that provider*, chosen when it is registered, not as a daemon-wide
//! switch that would re-open the hole for every other provider at the
//! same time.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UrlRefusal {
    #[error("that is not a URL")]
    Unparseable,
    #[error("a provider URL must be https:// (or http:// for a local endpoint you allow)")]
    BadScheme,
    #[error("a provider URL must name a host")]
    NoHost,
    #[error(
        "put the credential in the key field, not in the URL — a URL is stored \
         and written to the Ledger, and cannot be unwritten"
    )]
    Userinfo,
    #[error(
        "that address is on this machine or this network. If you meant to, \
         register it as a local endpoint"
    )]
    NotPublic,
}

/// Whether a provider may point at this machine or this network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locality {
    /// The default: public internet only.
    #[default]
    PublicOnly,
    /// The user said "yes, this one is my own box".
    LocalAllowed,
}

/// Check a base URL at registration time.
///
/// Returns the parsed URL so the caller stores what was actually
/// validated rather than the original string.
pub fn validate_base_url(raw: &str, locality: Locality) -> Result<Url, UrlRefusal> {
    let url = Url::parse(raw.trim()).map_err(|_| UrlRefusal::Unparseable)?;

    // Userinfo first: it is the one refusal whose reason the user needs
    // to hear even if the URL is otherwise fine (#109). A password here
    // is persisted to a file and appended to the Ledger, which is
    // append-only — there is no unwriting it.
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UrlRefusal::Userinfo);
    }

    match url.scheme() {
        "https" => {}
        // Plaintext is only ever acceptable to something the user has
        // said is their own machine. Over the internet it would mean
        // prompts leaving in the clear while the Ledger records the same
        // `remote.` marking as a TLS request.
        "http" if locality == Locality::LocalAllowed => {}
        _ => return Err(UrlRefusal::BadScheme),
    }

    let Some(host) = url.host() else {
        return Err(UrlRefusal::NoHost);
    };
    if locality == Locality::LocalAllowed {
        return Ok(url);
    }
    match host {
        url::Host::Ipv4(ip) => {
            if !is_public(IpAddr::V4(ip)) {
                return Err(UrlRefusal::NotPublic);
            }
        }
        url::Host::Ipv6(ip) => {
            if !is_public(IpAddr::V6(ip)) {
                return Err(UrlRefusal::NotPublic);
            }
        }
        // A name cannot be judged here — that is what the resolver
        // guard is for. These two are worth refusing early anyway, so
        // the user gets a sentence instead of a connection error.
        url::Host::Domain(d) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            if d == "localhost" || d.ends_with(".localhost") {
                return Err(UrlRefusal::NotPublic);
            }
        }
    }
    Ok(url)
}

/// Is this address on the public internet?
///
/// Written out rather than leaning on `is_global`, which is still
/// unstable — and written to fail closed: anything not recognised as
/// ordinary public space is refused.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4 address wearing an IPv6 hat is still that address;
            // `::ffff:127.0.0.1` must not walk past the v4 rules.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public_v4(mapped);
            }
            is_public_v6(v6)
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_documentation()
        // 100.64.0.0/10 — carrier-grade NAT, i.e. the ISP's own network.
        || (a == 100 && (64..128).contains(&b))
        // 192.0.0.0/24 — IETF protocol assignments.
        || (a == 192 && b == 0 && ip.octets()[2] == 0)
        // 198.18.0.0/15 — benchmarking.
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4 — reserved.
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // fc00::/7 unique-local, fe80::/10 link-local.
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        // 2001:db8::/32 documentation.
        || (first == 0x2001 && ip.segments()[1] == 0x0db8))
}

/// A `reqwest` resolver that will not hand back a non-public address.
///
/// This is the half that survives DNS: the name is resolved here, at the
/// moment of the connection, and every answer is filtered. A host that
/// looked public at registration and answers `127.0.0.1` now gets no
/// addresses at all, and the request fails instead of reaching a
/// loopback service with the user's credential attached.
#[derive(Debug, Default, Clone)]
pub struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            // Port 0: reqwest substitutes the URL's port, or the
            // scheme's default, over whatever comes back here.
            let resolved = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let allowed: Vec<std::net::SocketAddr> =
                resolved.filter(|addr| is_public(addr.ip())).collect();
            if allowed.is_empty() {
                // Deliberately the same shape as an ordinary resolution
                // failure. There is nothing useful to tell the far side,
                // and the Ledger records the refusal.
                return Err(format!("{host} does not resolve to a public address").into());
            }
            Ok(Box::new(allowed.into_iter()) as Box<dyn Iterator<Item = _> + Send>)
        })
    }
}

/// A URL with any credential removed, for anything that persists or
/// displays it (#109).
///
/// `validate_base_url` refuses userinfo outright, so this exists for the
/// rows already on disk from before that check — and because the Ledger
/// is append-only, which makes "we will never write it" a promise that
/// has to hold at every call site rather than at one.
pub fn redact(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(mut url) => {
            if url.username().is_empty() && url.password().is_none() {
                return raw.to_string();
            }
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.to_string()
        }
        Err(_) => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #92, as filed: every one of these returned `Ok(())` and the
    /// broker really dialled them, forwarding the stored credential.
    #[test]
    fn the_filed_ssrf_targets_are_all_refused() {
        for evil in [
            "http://127.0.0.1:1/v1",
            "http://localhost:8080/v1",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]:9999/v1",
            "http://10.0.0.1/v1",
            "https://192.168.1.1/v1",
            "https://172.16.0.5/v1",
            "https://[fe80::1]/v1",
            "https://[::ffff:127.0.0.1]/v1",
            "https://100.64.0.1/v1",
        ] {
            assert!(
                validate_base_url(evil, Locality::PublicOnly).is_err(),
                "{evil} was accepted"
            );
        }
    }

    /// And the ordinary case still works — a check that refuses
    /// everything is not a fix.
    #[test]
    fn real_provider_endpoints_are_accepted() {
        for good in [
            "https://api.openai.com/v1",
            "https://api.anthropic.com",
            "https://llm.corp.example/v1",
            "https://8.8.8.8/v1",
        ] {
            assert!(
                validate_base_url(good, Locality::PublicOnly).is_ok(),
                "{good} was refused"
            );
        }
    }

    /// Issue #109: a password in the URL is persisted and written to an
    /// append-only Ledger. It is refused at the door, with a sentence
    /// that says where the credential should go instead.
    #[test]
    fn a_credential_in_the_url_is_refused_even_for_a_local_endpoint() {
        assert_eq!(
            validate_base_url(
                "https://alice:hunter2@llm.corp.example/v1",
                Locality::PublicOnly
            ),
            Err(UrlRefusal::Userinfo)
        );
        // Including where everything else would have been allowed —
        // "it is my own box" is not a reason to write a password to the
        // Ledger forever.
        assert_eq!(
            validate_base_url(
                "http://alice:hunter2@localhost:8080/v1",
                Locality::LocalAllowed
            ),
            Err(UrlRefusal::Userinfo)
        );
        // A bare username is userinfo too.
        assert_eq!(
            validate_base_url("https://alice@llm.corp.example/v1", Locality::PublicOnly),
            Err(UrlRefusal::Userinfo)
        );
    }

    /// Plaintext is for a machine the user has vouched for, never for
    /// the internet: otherwise prompts leave in the clear while the
    /// Ledger records the same `remote.` marking as a TLS request.
    #[test]
    fn http_is_only_for_an_endpoint_the_user_vouched_for() {
        assert_eq!(
            validate_base_url("http://api.openai.com/v1", Locality::PublicOnly),
            Err(UrlRefusal::BadScheme)
        );
        assert!(validate_base_url("http://192.168.1.50:11434/v1", Locality::LocalAllowed).is_ok());
        // Still not any scheme at all.
        for weird in ["file:///etc/passwd", "ftp://x/", "gopher://x/", "data:,hi"] {
            assert!(
                validate_base_url(weird, Locality::LocalAllowed).is_err(),
                "{weird} was accepted"
            );
        }
    }

    #[test]
    fn a_local_endpoint_is_allowed_when_the_user_says_so() {
        for ok in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:8080/v1",
            "http://192.168.1.50:11434/v1",
            "https://nas.lan/v1",
        ] {
            assert!(
                validate_base_url(ok, Locality::LocalAllowed).is_ok(),
                "{ok} was refused for a vouched-for endpoint"
            );
        }
    }

    /// The classification itself, at its edges. `::ffff:10.0.0.1` is the
    /// one people forget: a v4 private address wearing a v6 hat.
    #[test]
    fn address_classification_holds_at_the_boundaries() {
        let public = ["8.8.8.8", "1.1.1.1", "2606:4700::1111", "9.255.255.255"];
        let private = [
            "127.0.0.1",
            "0.0.0.0",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "169.254.169.254",
            "224.0.0.1",
            "255.255.255.255",
            "100.64.0.1",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd00::1",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
        ];
        for p in public {
            assert!(is_public(p.parse().unwrap()), "{p} was called private");
        }
        for p in private {
            assert!(!is_public(p.parse().unwrap()), "{p} was called public");
        }
        // 172.32 is outside the /12 and is genuinely public.
        assert!(is_public("172.32.0.1".parse().unwrap()));
        assert!(is_public("11.0.0.1".parse().unwrap()));
    }

    #[test]
    fn redaction_removes_the_credential_and_leaves_the_rest() {
        assert_eq!(
            redact("https://alice:hunter2@llm.corp.example/v1"),
            "https://llm.corp.example/v1"
        );
        assert_eq!(
            redact("https://api.openai.com/v1"),
            "https://api.openai.com/v1"
        );
        // Nothing parseable to redact: returned as-is rather than
        // dropped, so a malformed row is still visible in the audit.
        assert_eq!(redact("not a url"), "not a url");
        assert!(!redact("https://alice:hunter2@x.example/v1").contains("hunter2"));
    }
}
