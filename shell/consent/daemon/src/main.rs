//! lisa-consentd — the desktop consent surface, as an executable of its
//! own (issue #289, ADR-0035 §4, ADR-0030, ADR-0033, PLAN §5.10).
//!
//! # Why this is a Rust binary and not the GJS file next door
//!
//! agentd will only let the human's dialog release a destructive call,
//! and it decides who that is from two facts the transport supplies:
//! who owns `dev.lisaos.Consent1` (the broker's answer to
//! `GetNameOwner`) and what program is behind that connection
//! (`/proc/<pid>/exe`, through the broker's pidfd). Both, because #289
//! showed each alone is forgeable — `session.conf` ships
//! `<allow own="*"/>`, so the name goes to whoever asked first.
//!
//! The program half was the one still open. The dialog shipped as
//! `Exec=/usr/bin/lisa-app consent/lisa-consentd.js`, and `lisa-app`
//! ends in `exec gjs -m "$found"`, so the kernel's answer for the
//! consent surface was `/usr/bin/gjs-console` — verified on the
//! reference device, pid 18669. An **interpreter** on an allowlist
//! authorises every program that interpreter can run. A hostile GJS
//! script that `fork()`s and `exec`s `gjs` gets a fresh pid, so it also
//! steps around the same-process check, and it then satisfies every
//! question agentd knows how to ask.
//!
//! A native launcher that `exec`s the GJS surface fixes nothing: after
//! `execve` the exe is `gjs` again. **The process that owns the bus name
//! has to be the binary.** So this one does — it owns the name, it
//! subscribes to agentd's signals, it makes the `Confirm` call, and it
//! spawns the GJS dialog as a *child* over a pipe, purely to draw a
//! window and report what was clicked.
//!
//! The child has no session bus address (`renderer::STRIPPED_ENV`), so
//! it cannot own a name or call a method even if it wanted to. The
//! separation is a property of the child's environment rather than a
//! convention about who calls what.
//!
//! # What this process must never grow
//!
//! No model. No prompt entry. No tool calls of its own. Its only inputs
//! are agentd's `ConfirmationRequested`/`RefusalReported` signals and a
//! human's click arriving up a pipe it opened; its only output is
//! `dev.lisaos.Agent1.Confirm`. The moment it can be driven by generated
//! text it stops being a second pair of eyes (ADR-0030: anything
//! reachable from inside is not a guardrail).
//!
//! It deliberately exposes **no** D-Bus method that approves anything. A
//! peer that could ask this daemon to approve something would be able to
//! launder its own request through it, which is the hole being closed.
//! The only approver is the pointer.

mod protocol;
mod renderer;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use protocol::{FromRenderer, ToRenderer, confirm_for};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const CONSENT_BUS_NAME: &str = "dev.lisaos.Consent1";
const CONSENT_OBJECT_PATH: &str = "/dev/lisaos/Consent1";

const AGENT_BUS: &str = "dev.lisaos.Agent1";
const AGENT_PATH: &str = "/dev/lisaos/Agent1";
const AGENT_IFACE: &str = "dev.lisaos.Agent1";

const VERSION: &str = concat!("lisa-consentd ", env!("CARGO_PKG_VERSION"));

/// Call ids with a window on screen. Shared with the D-Bus object so
/// `PendingCount` can report it.
type Open = Arc<Mutex<HashSet<u64>>>;

/// The read-only face of this daemon on the bus.
///
/// There is no `Approve()` here on purpose, and adding one would undo
/// the whole of #145 and #289: a peer that can ask the dialog to say yes
/// has laundered its own request through the one connection agentd
/// trusts.
struct Consent1 {
    open: Open,
}

#[zbus::interface(name = "dev.lisaos.Consent1")]
impl Consent1 {
    /// Names the *program*, not the script, so an operator reading
    /// `busctl` output can tell the shipped daemon from a squatter that
    /// learned to answer `Ping`.
    fn ping(&self) -> String {
        VERSION.to_string()
    }

    /// How many confirmations are on screen. For tests and for a status
    /// line; it reveals a count, never a call's contents.
    fn pending_count(&self) -> u32 {
        self.open.lock().expect("open lock").len() as u32
    }
}

/// The dialog child and the pipe to it.
struct Dialogs {
    child: Option<tokio::process::Child>,
    stdin: Option<tokio::process::ChildStdin>,
    answers: tokio::sync::mpsc::UnboundedSender<FromRenderer>,
    open: Open,
    /// The accessibility bus, resolved on OUR session connection and
    /// handed to the child, which has no session bus of its own.
    /// Re-resolved per spawn: `at-spi-bus-launcher` starts on demand, so
    /// asking once at startup would answer for a session that had not
    /// launched it yet.
    a11y: Option<zbus::Connection>,
}

impl Dialogs {
    fn new(
        answers: tokio::sync::mpsc::UnboundedSender<FromRenderer>,
        open: Open,
        a11y: Option<zbus::Connection>,
    ) -> Dialogs {
        Dialogs {
            child: None,
            stdin: None,
            answers,
            open,
            a11y,
        }
    }

    /// Has the renderer we spawned exited?
    ///
    /// Checked before every send rather than only on a write error,
    /// because a dead child's pipe accepts a write into the buffer and
    /// then drops it — which would look, from here, exactly like a
    /// dialog nobody clicked.
    fn child_is_gone(&mut self) -> bool {
        match self.child.as_mut() {
            None => true,
            Some(c) => matches!(c.try_wait(), Ok(Some(_)) | Err(_)),
        }
    }

    /// Start the dialog process if it is not running.
    ///
    /// Spawned lazily: this daemon is D-Bus-activated the moment a modal
    /// parks, and a window process that idles from login to the first
    /// confirmation is a window process that can be killed before the
    /// confirmation arrives.
    async fn ensure(&mut self) -> anyhow::Result<()> {
        if !self.child_is_gone() {
            return Ok(());
        }
        // A child that died takes its unanswered dialogs with it. The
        // calls behind them stay parked in agentd and expire, which is
        // the safe direction: nothing is approved by a crash.
        self.open.lock().expect("open lock").clear();

        let script = renderer::renderer_path();
        let a11y = match &self.a11y {
            Some(conn) => a11y_bus_address(conn).await,
            None => None,
        };
        let mut cmd = renderer::renderer_command(
            std::path::Path::new(renderer::GJS),
            &script,
            a11y.as_deref(),
        );
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("spawning {} {}: {e}", renderer::GJS, script.display()))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        let answers = self.answers.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match FromRenderer::from_line(&line) {
                    Ok(a) => {
                        if answers.send(a).is_err() {
                            return;
                        }
                    }
                    // Loudly ignored. A renderer saying things we do not
                    // understand is a renderer that has been replaced,
                    // and the safe answer is no answer at all.
                    Err(e) => eprintln!("lisa-consentd: unreadable answer from the dialog: {e}"),
                }
            }
        });

        self.child = Some(child);
        self.stdin = Some(stdin);
        Ok(())
    }

    /// Put something on screen.
    async fn show(&mut self, msg: ToRenderer) -> anyhow::Result<()> {
        let call_id = match &msg {
            ToRenderer::Confirm { call_id, .. } | ToRenderer::Refusal { call_id, .. } => *call_id,
        };
        // agentd re-emits on reconnect; one dialog per call.
        if self.open.lock().expect("open lock").contains(&call_id) {
            return Ok(());
        }
        self.ensure().await?;
        let stdin = self.stdin.as_mut().expect("spawned stdin");
        stdin.write_all(msg.to_line().as_bytes()).await?;
        stdin.flush().await?;
        self.open.lock().expect("open lock").insert(call_id);
        Ok(())
    }

    /// A dialog has been answered; it is no longer on screen.
    fn close(&self, call_id: u64) -> bool {
        self.open.lock().expect("open lock").remove(&call_id)
    }
}

/// Where the accessibility bus is, asked on our session connection.
///
/// The renderer cannot ask this for itself — it has no session bus, by
/// design — so the parent asks and passes the answer down. Best effort:
/// a session with no `org.a11y.Bus` (a11y switched off, a bare session)
/// gets `None`, and the dialog still draws.
async fn a11y_bus_address(conn: &zbus::Connection) -> Option<String> {
    conn.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.a11y.Bus"),
        "GetAddress",
        &(),
    )
    .await
    .ok()?
    .body()
    .deserialize::<String>()
    .ok()
}

/// Tell agentd what the person clicked.
///
/// This call goes out on **this** process's connection, which is the
/// entire point: agentd reads `/proc/<pid>/exe` for the peer that calls
/// `Confirm`, and the answer is `/usr/bin/lisa-consentd`.
async fn confirm(conn: &zbus::Connection, call_id: u64, approve: bool) {
    let reply = conn
        .call_method(
            Some(AGENT_BUS),
            AGENT_PATH,
            Some(AGENT_IFACE),
            "Confirm",
            &(call_id, approve),
        )
        .await;
    if let Err(e) = reply {
        // A refused `Confirm` is worth a log line and nothing more: the
        // call stays parked and expires, which is the safe direction.
        eprintln!("lisa-consentd: Confirm({call_id}, {approve}) failed: {e}");
    }
}

/// One signal from agentd, as something to draw — or nothing.
///
/// Separated from the stream so the member/body mapping is a pure
/// function with a test. Anything that is not one of the two signals we
/// know is dropped: this surface renders what agentd asks for and
/// nothing it merely happens to overhear.
fn to_renderer(member: &str, call_id: u64, payload: String) -> Option<ToRenderer> {
    match member {
        "ConfirmationRequested" => Some(ToRenderer::Confirm {
            call_id,
            spec: payload,
        }),
        "RefusalReported" => Some(ToRenderer::Refusal {
            call_id,
            report: payload,
        }),
        _ => None,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let open: Open = Arc::new(Mutex::new(HashSet::new()));

    // Serve the object and subscribe FIRST, own the name LAST (#244).
    // agentd starts this process with `StartServiceByName`, which
    // returns as soon as the broker says the name is owned, and emits
    // `ConfirmationRequested` immediately afterwards. So owning the name
    // has to mean "the dialog is listening", or the very first prompt is
    // emitted into a session where nobody is subscribed and the call
    // sits parked until it expires.
    let conn = zbus::connection::Builder::session()?
        .serve_at(CONSENT_OBJECT_PATH, Consent1 { open: open.clone() })?
        .build()
        .await?;

    // Sender-filtered: the broker resolves `dev.lisaos.Agent1` to its
    // current owner, so a peer that merely emits a signal with the right
    // interface cannot put a fabricated confirmation on screen. Without
    // this, anything on the session bus could describe a real parked
    // call as something harmless and collect a click for it.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(AGENT_BUS)?
        .interface(AGENT_IFACE)?
        .path(AGENT_PATH)?
        .build();
    let mut signals = zbus::MessageStream::for_match_rule(rule, &conn, None).await?;

    let (answer_tx, mut answer_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut dialogs = Dialogs::new(answer_tx, open.clone(), Some(conn.clone()));

    // `DoNotQueue`, and no `AllowReplacement`: if something already
    // holds the consent name this process must fail loudly rather than
    // sit in a queue looking like it is running, and once we hold it
    // nothing may take it away. Two surfaces for one call is worse than
    // one, and agentd trusts only the current owner.
    let reply = conn
        .request_name_with_flags(
            CONSENT_BUS_NAME,
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        )
        .await?;
    match reply {
        zbus::fdo::RequestNameReply::PrimaryOwner => {
            eprintln!("lisa-consentd: owning {CONSENT_BUS_NAME}");
        }
        other => {
            anyhow::bail!(
                "{CONSENT_BUS_NAME} is already owned ({other:?}) — refusing to run a second \
                 consent surface"
            );
        }
    }

    loop {
        tokio::select! {
            msg = signals.next() => {
                let Some(Ok(msg)) = msg else { break };
                let header = msg.header();
                let Some(member) = header.member().map(|m| m.to_string()) else { continue };
                let Ok((call_id, payload)) = msg.body().deserialize::<(u64, String)>() else {
                    continue;
                };
                let Some(work) = to_renderer(&member, call_id, payload) else { continue };
                if let Err(e) = dialogs.show(work).await {
                    // The operator's problem to see. Nothing is
                    // approved: the call stays parked in agentd and can
                    // only be withdrawn or expire (#244).
                    eprintln!("lisa-consentd: no dialog for call {call_id}: {e}");
                }
            }
            answer = answer_rx.recv() => {
                let Some(FromRenderer { call_id, answer }) = answer else { break };
                // An answer for a dialog we did not open is discarded.
                // The renderer cannot invent a call id and have it
                // reach agentd.
                if !dialogs.close(call_id) {
                    eprintln!("lisa-consentd: dialog answered call {call_id}, which is not open");
                    continue;
                }
                if let Some(approve) = confirm_for(answer) {
                    confirm(&conn, call_id, approve).await;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::Answer;

    #[test]
    fn only_agentds_two_signals_draw_anything() {
        assert_eq!(
            to_renderer("ConfirmationRequested", 1, "{}".into()),
            Some(ToRenderer::Confirm {
                call_id: 1,
                spec: "{}".into()
            })
        );
        assert_eq!(
            to_renderer("RefusalReported", 1, "{}".into()),
            Some(ToRenderer::Refusal {
                call_id: 1,
                report: "{}".into()
            })
        );
        for member in ["Confirm", "CallCompleted", "NameOwnerChanged", ""] {
            assert!(
                to_renderer(member, 1, "{}".into()).is_none(),
                "`{member}` put a window on screen"
            );
        }
    }

    /// A refusal report can never become an approval, at any layer. The
    /// protocol says so (`confirm_for`), and this is the assertion at
    /// the layer that would actually make the call.
    #[test]
    fn a_dismissed_refusal_makes_no_confirm_call() {
        assert_eq!(confirm_for(Answer::Dismiss), None);
    }

    /// The renderer answers dialogs; it does not name calls. An id we
    /// never opened is dropped before `Confirm` is reached — otherwise a
    /// compromised dialog could sweep the id space and approve calls
    /// nobody was shown.
    #[test]
    fn an_answer_for_an_unopened_call_is_dropped() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let open: Open = Arc::new(Mutex::new(HashSet::new()));
        let dialogs = Dialogs::new(tx, open.clone(), None);
        assert!(!dialogs.close(41), "an unopened call was accepted");
        open.lock().unwrap().insert(41);
        assert!(dialogs.close(41));
        assert!(!dialogs.close(41), "the same dialog answered twice");
    }
}
