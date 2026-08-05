//! HTTP/1.1 over a unix socket, for a caller that is forbidden IP
//! sockets outright (#288).
//!
//! ## Why this exists
//!
//! `lisa-harnessd` hosts the model. It ran with
//! `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6` for exactly one
//! reason: it called `lisa-inferenced` on `http://127.0.0.1:7778`.
//! Loopback was supposed to be pinned by `IPAddressDeny=any` +
//! `IPAddressAllow=localhost` — and **that pair is a no-op in a user
//! unit**. An IP firewall is a cgroup BPF program and `systemd --user`
//! cannot load one; it says so in the journal:
//!
//! ```text
//! lisa-agentd.service: unit configures an IP firewall, but not running as root.
//! ```
//!
//! Measured on the reference iMac with two transient *user* units:
//!
//! ```text
//! IPAddressDeny=any + RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
//!     -> curl http://example.com   HTTP 200      (reached the world)
//! IPAddressDeny=any + RestrictAddressFamilies=AF_UNIX
//!     -> curl http://example.com   rc=7          (blocked)
//! ```
//!
//! So the only directive that confines an unprivileged unit is
//! `RestrictAddressFamilies=`, because it is a seccomp filter on
//! `socket(2)` and needs no privilege. Taking `AF_UNIX` alone means the
//! `:7778` hop has to go — which it can, because inferenced **already**
//! serves the same OpenAI-compatible API on a unix socket
//! (`--socket %t/lisa/inferenced.sock`, the door lisa-contextd has used
//! since #163).
//!
//! ## How
//!
//! No new dependency, and no hand-rolled HTTP. `ureq` — already the
//! harness's client — has a pluggable transport layer, so the *only*
//! thing swapped out is "how the bytes get to the server". Chunked
//! transfer-encoding, keep-alive, header parsing and the SSE body
//! reader stay ureq's, which matters: the streaming lane is
//! `Transfer-Encoding: chunked`, so a hand-written `Connection: close`
//! client of the shape `lisa-contextd` uses for `/v1/embeddings` would
//! have had to grow a chunk decoder to stream tokens.
//!
//! ```no_run
//! # use forge_harness::unix_http;
//! let (agent, base) = unix_http::agent_for("unix:/run/user/1000/lisa/inferenced.sock");
//! let mut res = agent.get(format!("{base}/v1/models")).call().unwrap();
//! # let _ = res.body_mut().read_to_string();
//! ```
//!
//! ## Limits
//!
//! * `ureq::unversioned` is explicitly *not* covered by ureq's semver
//!   promise. A `ureq` minor bump can break this file; it cannot break
//!   it silently, because it will not compile.
//! * The authority in the request URI is a placeholder (`localhost`).
//!   It reaches the server as `Host: localhost`, which inferenced
//!   ignores — there is no name-based routing on the other end.
//! * Unix sockets have no TLS and want none: the peer is a file the
//!   kernel says is ours, `0600` under `$XDG_RUNTIME_DIR`.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use ureq::Error;
use ureq::config::Config;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::time::Duration as UreqDuration;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport,
};

/// The scheme that means "this is a filesystem path, not a host".
///
/// `unix:/run/user/1000/lisa/inferenced.sock` rather than a
/// `http+unix://` percent-encoded authority: the value is read by
/// humans in a unit file and in `--url`, and a path with `%2F` in it is
/// a path nobody can check at a glance.
pub const UNIX_SCHEME: &str = "unix:";

/// The socket path in a `unix:` URL, or `None` for anything else.
///
/// ```
/// # use forge_harness::unix_http::socket_path;
/// assert_eq!(socket_path("unix:/run/lisa/inferenced.sock").unwrap().to_str(),
///            Some("/run/lisa/inferenced.sock"));
/// assert!(socket_path("http://127.0.0.1:7778").is_none());
/// ```
pub fn socket_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix(UNIX_SCHEME)?;
    // `unix://<path>` is tolerated because somebody will type it: two
    // slashes read as an authority in every other scheme, and an empty
    // authority followed by an absolute path is the same socket.
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    if rest.is_empty() {
        return None;
    }
    Some(PathBuf::from(rest))
}

/// An agent for `url`, plus the base every request path hangs off.
///
/// * `unix:<path>` → an agent that speaks HTTP over that socket and
///   nothing else. It has no TCP connector at all, so a redirect to a
///   real host cannot quietly re-open the route this exists to close.
/// * anything else → ureq's defaults, unchanged, so a developer
///   pointing at `http://127.0.0.1:7778` on an unconfined host behaves
///   exactly as before.
pub fn agent_for(url: &str) -> (ureq::Agent, String) {
    match socket_path(url) {
        Some(path) => (unix_agent(&path), "http://localhost".to_string()),
        None => (
            ureq::Agent::new_with_defaults(),
            url.trim_end_matches('/').to_string(),
        ),
    }
}

/// An agent bound to one unix socket.
fn unix_agent(path: &Path) -> ureq::Agent {
    let config = Config::builder()
        // A proxy is a route to a host, and there is no host. Left to
        // the default this would read `HTTP_PROXY` out of the daemon's
        // environment and try to honour it.
        .proxy(None)
        .build();
    ureq::Agent::with_parts(config, UnixConnector::new(path), NoResolver)
}

/// There is nothing to resolve; the address is a path.
///
/// ureq resolves before it connects, and a resolver must return at
/// least one address, so this returns an unroutable placeholder that
/// [`UnixConnector`] ignores. Doing the lookup instead would be worse
/// than pointless: `getaddrinfo` is the one call in this path that can
/// still touch the network.
#[derive(Debug, Default)]
struct NoResolver;

impl Resolver for NoResolver {
    fn resolve(
        &self,
        _uri: &ureq::http::Uri,
        _config: &Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error> {
        let mut addrs = self.empty();
        addrs.push(SocketAddr::from(([0, 0, 0, 0], 0)));
        Ok(addrs)
    }
}

/// Opens [`UnixTransport`]s to one fixed socket.
struct UnixConnector {
    path: PathBuf,
}

impl UnixConnector {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }
}

impl fmt::Debug for UnixConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnixConnector")
            .field("path", &self.path)
            .finish()
    }
}

impl Connector<()> for UnixConnector {
    type Out = UnixTransport;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _chained: Option<()>,
    ) -> Result<Option<Self::Out>, Error> {
        // `connect(2)` on a unix socket either finds a listener or
        // fails at once with ENOENT/ECONNREFUSED, so it needs no
        // timeout of its own — the same reasoning lisa-contextd's
        // embedder records for the same socket.
        let stream = UnixStream::connect(&self.path)?;
        let config = &details.config;
        let buffers = LazyBuffers::new(config.input_buffer_size(), config.output_buffer_size());
        Ok(Some(UnixTransport {
            stream,
            buffers,
            read_timeout: None,
            write_timeout: None,
        }))
    }
}

/// One connected unix socket, presented to ureq as a byte pipe.
pub struct UnixTransport {
    stream: UnixStream,
    buffers: LazyBuffers,
    read_timeout: Option<UreqDuration>,
    write_timeout: Option<UreqDuration>,
}

impl fmt::Debug for UnixTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnixTransport")
            .field("peer", &self.stream.peer_addr().ok())
            .finish()
    }
}

/// Only call `setsockopt` when the deadline actually changed — ureq's
/// TCP transport does the same, and this loop runs once per read of a
/// streaming response.
fn maybe_set(
    timeout: NextTimeout,
    previous: &mut Option<UreqDuration>,
    stream: &UnixStream,
    set: impl Fn(&UnixStream, Option<std::time::Duration>) -> io::Result<()>,
) -> io::Result<()> {
    let wanted = timeout.not_zero();
    if wanted != *previous {
        set(stream, wanted.map(|t| *t))?;
        *previous = wanted;
    }
    Ok(())
}

/// A socket deadline surfaces as `WouldBlock` on Linux and `TimedOut`
/// elsewhere. ureq has to see [`Error::Timeout`] either way or a stalled
/// daemon reads as a transport failure with a confusing message.
fn timed_out(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

impl Transport for UnixTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        maybe_set(
            timeout,
            &mut self.write_timeout,
            &self.stream,
            UnixStream::set_write_timeout,
        )?;
        let output = &self.buffers.output()[..amount];
        match self.stream.write_all(output) {
            Ok(()) => Ok(()),
            Err(e) if timed_out(&e) => Err(Error::Timeout(timeout.reason)),
            Err(e) => Err(e.into()),
        }
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        maybe_set(
            timeout,
            &mut self.read_timeout,
            &self.stream,
            UnixStream::set_read_timeout,
        )?;
        let input = self.buffers.input_append_buf();
        let amount = match self.stream.read(input) {
            Ok(n) => n,
            Err(e) if timed_out(&e) => return Err(Error::Timeout(timeout.reason)),
            Err(e) => return Err(e.into()),
        };
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        // Pooling reuses a transport, so "is it still there" has to be
        // answered without consuming anything: a byte waiting on an
        // idle connection is the server having closed it (or sent
        // garbage), and either way it is not reusable.
        if self.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let mut probe = [0u8; 1];
        let open = matches!(
            self.stream.read(&mut probe),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock
        );
        if self.stream.set_nonblocking(false).is_err() {
            return false;
        }
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::os::unix::net::UnixListener;

    #[test]
    fn socket_path_reads_both_spellings() {
        assert_eq!(
            socket_path("unix:/run/lisa/inferenced.sock"),
            Some(PathBuf::from("/run/lisa/inferenced.sock"))
        );
        assert_eq!(
            socket_path("unix:///run/lisa/inferenced.sock"),
            Some(PathBuf::from("/run/lisa/inferenced.sock"))
        );
        assert_eq!(socket_path("unix:"), None);
        assert_eq!(socket_path("http://127.0.0.1:7778"), None);
        assert_eq!(socket_path(""), None);
    }

    #[test]
    fn agent_for_http_keeps_the_url_verbatim() {
        let (_agent, base) = agent_for("http://127.0.0.1:7778/");
        assert_eq!(base, "http://127.0.0.1:7778");
    }

    /// A one-request HTTP/1.1 server on a unix socket, answering with a
    /// **chunked** body — the framing the streaming lane actually uses,
    /// and the reason this went through ureq rather than a hand-rolled
    /// `Connection: close` client.
    fn chunked_server(path: PathBuf, body: &'static str) -> std::thread::JoinHandle<String> {
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = io::BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                request.push_str(&line);
            }
            let mut payload = vec![0u8; len];
            reader.read_exact(&mut payload).unwrap();
            let mut stream = stream;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
                .unwrap();
            for piece in body.as_bytes().chunks(7) {
                write!(stream, "{:x}\r\n", piece.len()).unwrap();
                stream.write_all(piece).unwrap();
                stream.write_all(b"\r\n").unwrap();
            }
            stream.write_all(b"0\r\n\r\n").unwrap();
            stream.flush().unwrap();
            request + &String::from_utf8_lossy(&payload)
        })
    }

    #[test]
    fn posts_and_reads_a_chunked_response_over_a_unix_socket() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("inferenced.sock");
        let server = chunked_server(
            sock.clone(),
            "data: {\"hello\":\"world\"}\n\ndata: [DONE]\n\n",
        );

        let url = format!("unix:{}", sock.display());
        let (agent, base) = agent_for(&url);
        assert_eq!(base, "http://localhost");
        let mut response = agent
            .post(format!("{base}/v1/chat/completions"))
            .send_json(serde_json::json!({"stream": true}))
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let text = response.body_mut().read_to_string().unwrap();

        let request = server.join().unwrap();
        assert!(request.contains("POST /v1/chat/completions"), "{request}");
        assert!(request.contains("\"stream\": true"), "{request}");
        // Reassembled across chunk boundaries the client never saw.
        assert_eq!(text, "data: {\"hello\":\"world\"}\n\ndata: [DONE]\n\n");
    }

    #[test]
    fn a_missing_socket_is_an_error_not_a_fallback_to_tcp() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("unix:{}", dir.path().join("absent.sock").display());
        let (agent, base) = agent_for(&url);
        let err = agent.get(format!("{base}/v1/models")).call().unwrap_err();
        assert!(
            matches!(err, Error::Io(_)),
            "expected the connect to fail, got {err:?}"
        );
    }
}
