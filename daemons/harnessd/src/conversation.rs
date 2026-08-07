//! Taint that outlives a single `Run` (#305).
//!
//! # The defect
//!
//! `bus_tools::Taint` is one-way and complete — nothing removes a tag —
//! and it was constructed fresh on every `Run`. That is the right rule at
//! the wrong scope. The model does not read a *run*, it reads a
//! **conversation**:
//!
//! 1. Turn 1: *"what does this page say?"*. Surfer answers with
//!    `provenance: "web"`, the run's taint gains `web`, and every bus
//!    call in that turn correctly goes out as `["user","web"]` and
//!    escalates.
//! 2. The turn ends. The surface appends the model's answer to its
//!    transcript.
//! 3. Turn 2: *"ok, do that"*. The same surface calls `Run` again with
//!    `options["history"]` carrying the prior turns — including the
//!    model's own restatement of the page.
//! 4. `Taint::new()`. Empty. The chain is `["user"]`, `tier::resolve`
//!    does not escalate, and `bus::grant_for` resolves `Trigger::Prompt`.
//!
//! Tool results are not replayed, so the raw page text does not literally
//! return; what returns is the model's restatement of it at full `user`
//! trust. That is model-laundered untrusted content, and ADR-0030's whole
//! premise is that we do not rely on the model declining to launder.
//!
//! # The scope, and what clears it
//!
//! The taint is carried on the **conversation**, and exactly one thing
//! clears it, which is not reachable from inside the process the model
//! runs in (CLAUDE.md rule 6a):
//!
//! - **A new conversation.** The person starting a fresh chat at a prompt
//!   surface is the authority, expressed as a `Run` whose history does
//!   not continue an existing one. A model that has read a hostile page
//!   cannot start a conversation; it emits tool calls into one.
//!
//! And one thing that must NOT clear it, which used to: **the surface
//! being closed and reopened.** The store was in-memory and keyed partly
//! on the caller's broker-assigned unique name, on the reasoning that "a
//! conversation the surface no longer holds is one nothing can continue
//! anyway". That reasoning was wrong about the shipped surface. The
//! Assistant persists every session and restores the most recent one
//! into its transcript on launch, so: read a hostile page, close the
//! window, reopen it, and the replayed transcript arrives under a new
//! unique name — a new key, an empty set, and turn N goes out on
//! `["user"]`. That is this module's own scenario across a relaunch, and
//! it is the plant-Monday-cash-Friday window the memory module names.
//! So the taint is keyed on the conversation alone and written to disk.
//!
//! There is deliberately **no method to clear a live conversation's
//! taint**. An API that forgets what a run has read is the exact
//! capability an injected instruction would ask for, and a guardrail with
//! an off switch reachable from the thing it guards is not a guardrail.
//! A person who wants a clean chain opens a new chat, which costs them
//! one click and costs a page everything.
//!
//! # How a conversation is identified, without asking the surface
//!
//! `Run` has no session parameter and the Assistant sends none: it
//! replays `[{role, content}]` per run and nothing else
//! (`shell/assistant/lisa-assistant.js`, `historyPayload`). A fix that
//! needed a new option would be a fix that does nothing until every
//! surface is changed — and the surfaces live in another repo
//! (ADR-0039).
//!
//! So the conversation is identified from what the surface already
//! sends: **the first user turn**. On turn 1 the history is empty and
//! the prompt *is* the first user turn; on every later turn that same
//! text is `history[0]`. The Assistant appends and never rewrites, so
//! the key is stable for the life of a chat — across relaunches
//! included, which is the point — and different for the next chat.
//!
//! # Why the key does not carry the owner, and why that is safe
//!
//! It used to (ADR-0033, rule 6b: a thing a later call can act on stores
//! its `Owner`). The unique name changes on every relaunch, so keying on
//! it meant the taint could not survive one — and the rule's purpose is
//! to stop one peer *gaining* something at another's expense. Nothing
//! here is gained:
//!
//! - Reading another conversation's set can only ADD tags to your own
//!   run. There is no operation that removes one.
//! - Writing to another's can only add tags to theirs — more
//!   confirmations for that person, never fewer.
//!
//! So the worst a hostile peer buys by colliding with a conversation is
//! over-escalation, at the price of losing its own trust. What keying on
//! the owner bought, by contrast, was a real escalation on every
//! relaunch. The residue is a weak side channel — a peer can learn that
//! *some* conversation opening with a given text is tainted, by paying
//! for the escalation itself — and that is the trade, stated rather than
//! hidden.
//!
//! Three honest limits:
//!
//! - Two conversations whose first message is identical (*"hi"*) share a
//!   taint set. That over-escalates, which is the safe direction.
//! - A surface that trims the head of its history changes the key and
//!   loses the taint. No shipped surface does; a surface that starts to
//!   is the moment to give `Run` a real session id, and this module gains
//!   one branch.
//! - The file is in the daemon's own state directory, so a same-user
//!   process can delete it. That is true of every user-scope state file
//!   we keep and is not what this defends against: the attacker in
//!   ADR-0030 is content the model read, which reaches no filesystem
//!   except through the confirmed, tiered tools.

use bus_tools::Taint;
use forge_harness::{Message, Role};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

/// How many conversations may hold their own taint set before the store
/// stops keeping them apart.
///
/// **Nothing is ever evicted.** The first version bounded the map by
/// evicting the oldest set, and the close-replay of #305 turned that
/// into a laundering primitive: with the key content-scoped and the
/// store persisted, any same-user peer could drive 256 fresh
/// conversations and evict a TARGETED victim's taint — under-escalation,
/// surviving restart, contradicting this module's own "no operation
/// removes a tag" claim. So at the cap the store degrades the other way:
/// new tags join a global overflow set that every conversation inherits.
/// Over-escalation for everyone — including whoever caused it — which is
/// loud, safe, and worthless as an attack. A person recovers by pruning
/// the store file from outside the loop (rule 6a: that door belongs to
/// the person, not the model).
pub const MAX_CONVERSATIONS: usize = 4096;

/// The identity of the conversation a run belongs to.
///
/// Opaque on purpose, and deliberately NOT scoped to the caller — see
/// the module docs: the owner is a per-connection name that changes
/// when the surface relaunches, and a taint that resets on relaunch is
/// the defect this module exists to close.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ConversationKey(String);

/// The conversation `prompt` belongs to.
///
/// `history` is what the surface replayed; `prompt` is this turn's text.
/// The first *user* message of the conversation names it — taken from
/// the history when there is one, and from the prompt when there is not,
/// because a run with no history is that first turn.
pub fn key_for(history: &[Message], prompt: &str) -> ConversationKey {
    let first = history
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
        .unwrap_or(prompt);
    ConversationKey(format!("{:016x}", fingerprint(first)))
}

/// Where the store lives: beside the memory database, in the user's own
/// data directory. `None` when neither `XDG_DATA_HOME` nor `HOME` is
/// set, which is the same soft failure `memory::store_path` takes — the
/// taint then lasts as long as the daemon, which is where it started.
pub fn store_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|h| h.join(".local/share"))
        })?;
    Some(base.join("lisa").join("conversation-taint.json"))
}

/// FNV-1a over the first user turn.
///
/// Not a security hash and not asked to be one. A collision makes two
/// conversations share a taint set, which can only ADD tags to a chain —
/// the same asymmetry every other part of this design leans on. Written
/// out rather than using `DefaultHasher` because a hash whose stability
/// the standard library does not promise is a hash that can silently
/// change what "the same conversation" means between builds.
fn fingerprint(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The provenance an attached content part contributes on arrival.
///
/// `options["attachments"]` forwards a screenshot or a scanned document
/// verbatim into the model's context and used to contribute **nothing**
/// (#305, "adjacent, same shape") — so a screenshot of a hostile page was
/// the same attack with no tag anywhere, even though `screen` and `file`
/// are both first-class untrusted classes in agentd's `tier.rs`.
///
/// An image is `screen`; anything else — an extracted document, an audio
/// part, whatever a provider invents next — is `file`. The prompt is not
/// an attachment: it arrives as its own argument and keeps the trigger's
/// class. Neither answer is trusted, so a part this guesses wrong about
/// still costs the run the right amount of trust.
pub fn attachment_provenance(part: &Value) -> &'static str {
    match part.get("type").and_then(Value::as_str) {
        Some("image_url") | Some("input_image") => "screen",
        _ => "file",
    }
}

/// What every remembered conversation has read.
///
/// Backed by a file when one is available: the surface outlives this
/// process (it restores its transcript on launch), so the taint has to
/// as well, or closing a window launders everything read in it (#305).
#[derive(Default)]
pub struct TaintStore {
    inner: Mutex<Inner>,
    /// Where to persist. `None` keeps the old in-memory behaviour, which
    /// is what tests of the pure logic want and what a machine with no
    /// writable data directory gets.
    path: Option<std::path::PathBuf>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Inner {
    tags: HashMap<ConversationKey, BTreeSet<String>>,
    /// Tags that arrived after the map hit [`MAX_CONVERSATIONS`]. Never
    /// emptied by anything in this process; applied to EVERY
    /// conversation while non-empty. The degraded mode, not a feature.
    #[serde(default)]
    overflow: BTreeSet<String>,
}

impl TaintStore {
    /// Open the store at `path`, loading whatever a previous run left.
    ///
    /// Fails soft in one direction only: an unreadable or corrupt file
    /// starts an EMPTY store rather than refusing to run — and says so,
    /// because that is a conversation getting its trust back and the
    /// person deserves to see the line. It never fails soft the other
    /// way: nothing here can turn a tag off.
    pub fn open_at(path: std::path::PathBuf) -> TaintStore {
        let inner = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Inner>(&bytes) {
                Ok(inner) => inner,
                Err(e) => {
                    eprintln!(
                        "harnessd: conversation taint at {} is unreadable ({e}); \
                         starting empty — conversations from before now are \
                         trusted again",
                        path.display()
                    );
                    Inner::default()
                }
            },
            // Not an error worth a line: the first run on a machine has
            // no file, which is the overwhelmingly common case.
            Err(_) => Inner::default(),
        };
        TaintStore {
            inner: Mutex::new(inner),
            path: Some(path),
        }
    }

    /// Write the map out, atomically. Best-effort and loud: losing the
    /// write means a relaunch forgets, which is exactly #305's defect,
    /// so it must not be silent.
    fn persist(&self, inner: &Inner) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            let bytes = serde_json::to_vec(inner)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            eprintln!(
                "harnessd: could not persist conversation taint to {} ({e}); \
                 this conversation's taint will not survive a restart",
                path.display()
            );
        }
    }
    /// The `Taint` this run starts with: everything its conversation has
    /// already read, plus whatever arrived attached to this turn.
    ///
    /// `convo` is `None` when the transport could not identify the caller
    /// at all. Such a caller's ceiling is already `Trigger::Event`
    /// (`caller::ceiling`), so its chain carries an untrusted class on
    /// every call regardless; it gets a per-run taint, as before.
    pub fn open(&self, convo: Option<&ConversationKey>, attachments: &[Value]) -> Taint {
        let taint = Taint::new();
        if let Some(key) = convo {
            let inner = self.inner.lock().expect("taint store lock");
            if let Some(tags) = inner.tags.get(key) {
                for tag in tags {
                    taint.add(tag);
                }
            }
            // Degraded mode: past the cap, unattributable tags belong
            // to everyone. Over-escalation is the only safe reading of
            // "we no longer know whose this was".
            for tag in &inner.overflow {
                taint.add(tag);
            }
        }
        for part in attachments {
            taint.add(attachment_provenance(part));
        }
        taint
    }

    /// Fold what the run read back onto its conversation.
    ///
    /// Union only. A run can add to its conversation's taint and can
    /// never take anything off it — the same one-way rule `Taint` itself
    /// keeps, one scope up, and the reason there is no `forget` here.
    pub fn close(&self, convo: Option<&ConversationKey>, taint: &Taint) {
        let Some(key) = convo else {
            return;
        };
        let tags = taint.tags();
        if tags.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().expect("taint store lock");
        if inner.tags.contains_key(key) || inner.tags.len() < MAX_CONVERSATIONS {
            inner.tags.entry(key.clone()).or_default().extend(tags);
        } else {
            // The cap. Nothing is evicted — eviction was a laundering
            // primitive (#305 replay) — so the new tags go global and
            // every conversation pays until a person prunes the store.
            eprintln!(
                "harnessd: conversation-taint store is at its cap of \
                 {MAX_CONVERSATIONS}; new conversations now escalate \
                 EVERY conversation. Prune {} (from outside the agent) \
                 to restore per-conversation taint.",
                self.path
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "the in-memory store".into())
            );
            inner.overflow.extend(tags);
        }
        // Under the lock on purpose: a write that races another run's
        // write could otherwise persist the older map and drop a tag.
        self.persist(&inner);
    }

    /// What a conversation has read, and how many conversations are
    /// remembered.
    ///
    /// Test-only, and deliberately so: nothing in the daemon needs to
    /// *read* the store outside [`TaintStore::open`], and a getter with
    /// no production caller is a getter a future one will reach for
    /// without asking whether a taint should be readable at all. They
    /// exist so the invariants below are assertions rather than claims.
    #[cfg(test)]
    pub fn tags_of(&self, convo: &ConversationKey) -> Vec<String> {
        self.inner
            .lock()
            .expect("taint store lock")
            .tags
            .get(convo)
            .map(|t| t.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().expect("taint store lock").tags.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_harness::{ToolCall, ToolProvider};
    use serde_json::json;
    use std::sync::{Arc, Mutex as StdMutex};

    fn user(text: &str) -> Message {
        Message::user(text.to_string())
    }

    fn assistant(text: &str) -> Message {
        Message::assistant_text(text.to_string())
    }

    /// A `BusTransport` that records the provenance chain each call went
    /// out with, so the assertions below read the chain the SHIPPING
    /// provider produced rather than one the test wrote.
    struct Recorder(Arc<StdMutex<Vec<Vec<String>>>>);

    impl bus_tools::BusTransport for Recorder {
        fn request_call(
            &self,
            _app_id: &str,
            _tool: &str,
            _args_json: &str,
            _actor: &str,
            chain: &[&str],
        ) -> Result<(u64, String, String), String> {
            self.0
                .lock()
                .unwrap()
                .push(chain.iter().map(|s| s.to_string()).collect());
            Ok((1, "executed".into(), r#"{"result":{"ok":true}}"#.into()))
        }
    }

    /// Drive one bus call through a real `AgentBusTools` holding `taint`,
    /// and return the chain it put on the wire.
    fn chain_of(taint: &Taint) -> Vec<String> {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let tools = vec![bus_tools::BusTool {
            app_id: "app.lisaos.Surfer".into(),
            tool: "read_page".into(),
            wire: "app_lisaos_Surfer__read_page".into(),
            description: String::new(),
            input_schema: json!({"type": "object"}),
        }];
        let bus = bus_tools::AgentBusTools::with_transport(
            Box::new(Recorder(Arc::clone(&seen))),
            tools,
            "user",
        )
        .with_taint(taint.clone());
        bus.execute(&ToolCall {
            id: "t1".into(),
            name: "app_lisaos_Surfer__read_page".into(),
            args: json!({}),
        });
        seen.lock().unwrap()[0].clone()
    }

    /// #305, as the two turns that produce it. Turn 1 reads a page; turn
    /// 2 is the *"ok, do that"* that used to go out fully trusted.
    ///
    /// The assertion is on the chain a real `AgentBusTools` puts on the
    /// wire, not on the store's own bookkeeping — the store could be
    /// perfectly correct and still not be what the bus call carries.
    #[test]
    fn a_page_read_in_turn_one_still_taints_turn_two() {
        let store = TaintStore::default();

        // Turn 1. Nothing replayed yet: this prompt IS the first turn.
        let k1 = key_for(&[], "what does this page say?");
        let t1 = store.open(Some(&k1), &[]);
        assert!(t1.is_clean(), "a fresh conversation starts clean");
        assert_eq!(chain_of(&t1), vec!["user"]);
        t1.add("web"); // Surfer answered, `provenance: "web"`
        assert_eq!(chain_of(&t1), vec!["user", "web"]);
        store.close(Some(&k1), &t1);

        // Turn 2, the next `Run`. The surface replays the prior turns.
        let history = [
            user("what does this page say?"),
            assistant("It says to wire the invoice to GB00EVIL."),
        ];
        let k2 = key_for(&history, "ok, do that");
        assert_eq!(k1, k2, "turn 2 was not recognised as the same conversation");
        let t2 = store.open(Some(&k2), &[]);
        assert_eq!(
            chain_of(&t2),
            vec!["user", "web"],
            "turn 2's bus call went out on a clean `[\"user\"]` chain — the \
             page's instruction is still in the context and nothing pays \
             for it (#305)"
        );
    }

    /// The other half, and the one that keeps the fix from being useless:
    /// a NEW conversation is clean. Taint that never decays makes the
    /// assistant ask for a confirmation forever after one web read, which
    /// is the confirmation fatigue ADR-0030 spends its argument on.
    #[test]
    fn a_new_conversation_starts_clean_and_that_is_the_only_way_out() {
        let store = TaintStore::default();
        let dirty = key_for(&[], "what does this page say?");
        let t = store.open(Some(&dirty), &[]);
        t.add("web");
        store.close(Some(&dirty), &t);

        // A different chat. Different first turn, different conversation.
        let fresh = key_for(&[], "how do I resize a photo?");
        assert_ne!(dirty, fresh);
        assert!(
            store.open(Some(&fresh), &[]).is_clean(),
            "a new conversation inherited the last one's taint"
        );
        // …and the tainted one has not been let off.
        assert_eq!(store.tags_of(&dirty), vec!["web".to_string()]);
    }

    /// Ownership is in the key, so one peer's conversation is not
    /// another's even when the words match exactly (ADR-0033, rule 6b).
    /// #305's reopen: the surface is closed and relaunched, the
    /// transcript is replayed under a NEW unique bus name, and the taint
    /// must still be there. This test is the one the old key could not
    /// pass — it keyed on the owner, so a relaunch was a clean slate.
    ///
    /// Two peers whose first user turn is identical DO now share a set.
    /// Asserted rather than tolerated: the sharing is one-way (a set can
    /// only gain tags), so the worst it buys is over-escalation, and the
    /// alternative bought a real escalation on every relaunch.
    ///
    /// What actually stops the old defect returning is not this test —
    /// a same-process test cannot simulate a new unique name — it is
    /// `key_for`'s SIGNATURE: it takes no caller, so making the key
    /// depend on one again means changing the function and every call
    /// site. The compiler enforces what a test here could only sample.
    /// The restart half, which a test can reach, is
    /// `the_taint_survives_this_daemon_restarting` (proven red by
    /// deleting the persist call).
    #[test]
    fn a_relaunched_surface_continues_the_same_tainted_conversation() {
        let store = TaintStore::default();
        // Before the relaunch: turn 1 reads a page.
        let before = key_for(&[], "what does this page say?");
        let t = store.open(Some(&before), &[]);
        t.add("web");
        store.close(Some(&before), &t);

        // After: a new process, a new unique name, the transcript
        // replayed. Same conversation, same taint.
        let after = key_for(
            &[
                user("what does this page say?"),
                assistant("It says to wire the invoice."),
            ],
            "ok, do that",
        );
        assert_eq!(before, after);
        assert_eq!(
            chain_of(&store.open(Some(&after), &[])),
            vec!["user", "web"]
        );
    }

    /// The taint only ever grows. A later run of the same conversation
    /// cannot hand back a smaller set and quietly narrow what the
    /// conversation has read — the asymmetry `Trigger::resolve` already
    /// uses, one level up.
    #[test]
    fn a_run_can_add_to_its_conversations_taint_and_never_subtract() {
        let store = TaintStore::default();
        let k = key_for(&[], "start");
        let first = store.open(Some(&k), &[]);
        first.add("web");
        first.add("mail");
        store.close(Some(&k), &first);

        // A run that read only `file` must not take `web` and `mail` off.
        let narrow = Taint::new();
        narrow.add("file");
        store.close(Some(&k), &narrow);
        assert_eq!(
            store.tags_of(&k),
            vec!["file".to_string(), "mail".to_string(), "web".to_string()]
        );
        // Nor does a clean run launder it.
        store.close(Some(&k), &Taint::new());
        assert_eq!(store.tags_of(&k).len(), 3);
    }

    /// A screenshot of a hostile page was the same attack with no tag
    /// anywhere: attachments went to the model verbatim and contributed
    /// nothing to the chain (#305, "adjacent, same shape").
    #[test]
    fn an_attachment_costs_the_run_its_trust_on_arrival() {
        let store = TaintStore::default();
        let k = key_for(&[], "what is in this?");
        let shot = json!({"type": "image_url",
                          "image_url": {"url": "data:image/png;base64,AAAA"}});
        let taint = store.open(Some(&k), std::slice::from_ref(&shot));
        assert_eq!(
            chain_of(&taint),
            vec!["user", "screen"],
            "a screenshot reached the model with nothing to pay for it"
        );

        // Anything that is not an image is content off a disk.
        let doc = json!({"type": "input_file", "file": {"file_data": "…"}});
        let taint = store.open(Some(&k), std::slice::from_ref(&doc));
        assert_eq!(chain_of(&taint), vec!["user", "file"]);

        // …and it persists into the next turn like everything else.
        store.close(Some(&k), &taint);
        assert_eq!(store.tags_of(&k), vec!["file".to_string()]);
    }

    /// The positive control for the whole module: a conversation that
    /// attaches nothing and reads nothing must cost nothing, or every
    /// turn escalates and the confirmation stops meaning anything.
    #[test]
    fn an_ordinary_conversation_never_leaves_the_user_chain() {
        let store = TaintStore::default();
        let k = key_for(&[], "what is 2 + 2?");
        let t = store.open(Some(&k), &[]);
        assert_eq!(chain_of(&t), vec!["user"]);
        store.close(Some(&k), &t);
        // Nothing was read, so nothing is stored — an empty entry per
        // conversation would spend the bound on silence.
        assert!(store.is_empty());
        let k2 = key_for(&[user("what is 2 + 2?")], "and 3 + 3?");
        assert_eq!(chain_of(&store.open(Some(&k2), &[])), vec!["user"]);
    }

    /// A caller the transport could not place gets a per-run taint, as
    /// before. Its ceiling is `Trigger::Event` anyway, so its chain
    /// already carries an untrusted class on every call.
    #[test]
    fn an_unidentifiable_caller_still_gets_a_working_run() {
        let store = TaintStore::default();
        let t = store.open(None, &[json!({"type": "image_url"})]);
        assert_eq!(t.tags(), vec!["screen".to_string()]);
        store.close(None, &t); // must not panic, must store nothing
        assert!(store.is_empty());
    }

    /// The #305 replay's laundering attack, refused: a same-user peer
    /// floods the store past its cap trying to evict a targeted
    /// conversation's taint. NOTHING is evicted — the victim keeps its
    /// tags — and the flood buys the degraded mode instead: overflow
    /// tags that every conversation inherits, the flooder's included.
    /// Under-escalation is the failure this store exists to prevent;
    /// over-escalation is the only acceptable shape for its limit.
    #[test]
    fn a_flood_cannot_evict_a_victims_taint_it_only_escalates_everyone() {
        let store = TaintStore::default();
        let victim = key_for(&[], "conversation 0");
        for i in 0..MAX_CONVERSATIONS + 10 {
            let k = key_for(&[], &format!("conversation {i}"));
            let t = Taint::new();
            t.add(if i == 0 { "web" } else { "screen" });
            store.close(Some(&k), &t);
        }
        // The victim's own tag survived the flood…
        assert!(
            store.tags_of(&victim).contains(&"web".to_string()),
            "the flood evicted the victim's taint — the laundering \
             primitive is back"
        );
        assert!(chain_of(&store.open(Some(&victim), &[])).contains(&"web".to_string()));
        // …and past the cap, a NEW conversation is not clean either:
        // the overflow tags reach it, so the flood bought escalation
        // for everyone rather than trust for anyone.
        let fresh = key_for(&[], "a genuinely new chat after the flood");
        assert!(
            !store.open(Some(&fresh), &[]).is_clean(),
            "past the cap, an unattributable conversation opened clean"
        );
    }

    /// Persistence, the other half of #305: harnessd restarts (an
    /// update, a crash, a logout) while the surface keeps its
    /// transcript. A store that only lived in memory handed those
    /// conversations their trust back.
    #[test]
    fn the_taint_survives_this_daemon_restarting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conversation-taint.json");
        let convo = key_for(&[], "read this for me");

        let first = TaintStore::open_at(path.clone());
        let t = first.open(Some(&convo), &[]);
        t.add("web");
        first.close(Some(&convo), &t);
        drop(first);

        // A different process entirely, same file.
        let second = TaintStore::open_at(path.clone());
        assert_eq!(
            chain_of(&second.open(Some(&convo), &[])),
            vec!["user", "web"],
            "a restart laundered what the conversation had read (#305)"
        );

        // A corrupt file starts empty rather than refusing to run — the
        // failure direction is stated in open_at and asserted here.
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(
            TaintStore::open_at(path).open(Some(&convo), &[]).is_clean(),
            "a corrupt store must not resurrect arbitrary tags"
        );
    }

    /// The key is the first USER turn, not the first turn of any role —
    /// an assistant message is the model's own text, and letting it name
    /// the conversation would put the identity of the taint bucket inside
    /// the thing the taint exists to contain.
    #[test]
    fn the_model_does_not_get_to_name_the_conversation() {
        let a = key_for(&[assistant("hello there"), user("the ask")], "next");
        let b = key_for(&[assistant("something else"), user("the ask")], "next");
        assert_eq!(a, b, "the model's own words changed the conversation key");
        let c = key_for(&[assistant("hello there"), user("other")], "next");
        assert_ne!(a, c);
    }
}
