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
//! The taint is carried on the **conversation**, and only two things
//! clear it. Neither is reachable from inside the process the model runs
//! in (CLAUDE.md rule 6a):
//!
//! - **A new conversation.** The person starting a fresh chat at a prompt
//!   surface is the authority, expressed as a `Run` whose history does
//!   not continue an existing one. A model that has read a hostile page
//!   cannot start a conversation; it emits tool calls into one.
//! - **A restart of this daemon**, which drops the whole store. Also not
//!   something a page can arrange, and honest to state: the taint is
//!   in-memory, because a conversation the surface no longer holds is one
//!   nothing can continue anyway.
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
//! So the conversation is identified from what the surface already sends:
//! **its owner plus the first user turn**. On turn 1 the history is empty
//! and the prompt *is* the first user turn; on every later turn that same
//! text is `history[0]`. The Assistant appends and never rewrites, so the
//! key is stable for the life of a chat and different for the next one.
//!
//! Two honest limits, both stated rather than papered over:
//!
//! - Two conversations whose first message is identical (*"hi"*) share a
//!   taint set. That over-escalates, which is the safe direction.
//! - A surface that trims the head of its history changes the key and
//!   loses the taint. No shipped surface does; a surface that starts to
//!   is the moment to give `Run` a real session id, and this module gains
//!   one branch.

use bus_tools::Taint;
use forge_harness::{Message, Role};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;

/// How many conversations keep a taint set at once.
///
/// Eviction LOSES taint, so this is generous rather than tight: the cost
/// of a big map is a few hundred short strings, and the cost of a small
/// one is a conversation that quietly gets its trust back. Forcing an
/// eviction means driving the surface into 256 fresh conversations,
/// which is a person's act, not a page's.
pub const MAX_CONVERSATIONS: usize = 256;

/// The identity of the conversation a run belongs to.
///
/// Opaque on purpose. It carries the **owner** — the broker-assigned
/// unique name of the caller — so one peer's conversation can never read
/// or write another's (ADR-0033, CLAUDE.md rule 6b): a different owner
/// produces a different key, which is ownership enforced by construction
/// rather than by a check somebody has to remember to write.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationKey(String);

/// The conversation `prompt` belongs to, for a caller the transport
/// identified as `owner`.
///
/// `history` is what the surface replayed; `prompt` is this turn's text.
/// The first *user* message of the conversation names it — taken from
/// the history when there is one, and from the prompt when there is not,
/// because a run with no history is that first turn.
pub fn key_for(owner: &str, history: &[Message], prompt: &str) -> ConversationKey {
    let first = history
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
        .unwrap_or(prompt);
    ConversationKey(format!("{owner}\u{1f}{:016x}", fingerprint(first)))
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

/// What every live conversation has read, keyed by owner + conversation.
#[derive(Default)]
pub struct TaintStore {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    tags: HashMap<ConversationKey, BTreeSet<String>>,
    /// Insertion order, for the bound. A conversation re-entered is not
    /// re-queued: the bound is on how many conversations are remembered,
    /// not on how busy they are.
    order: VecDeque<ConversationKey>,
}

impl TaintStore {
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
        if !inner.tags.contains_key(key) {
            inner.order.push_back(key.clone());
        }
        inner.tags.entry(key.clone()).or_default().extend(tags);
        while inner.order.len() > MAX_CONVERSATIONS {
            if let Some(oldest) = inner.order.pop_front() {
                inner.tags.remove(&oldest);
            }
        }
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

    const OWNER: &str = ":1.42";

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
        let k1 = key_for(OWNER, &[], "what does this page say?");
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
        let k2 = key_for(OWNER, &history, "ok, do that");
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
        let dirty = key_for(OWNER, &[], "what does this page say?");
        let t = store.open(Some(&dirty), &[]);
        t.add("web");
        store.close(Some(&dirty), &t);

        // A different chat. Different first turn, different conversation.
        let fresh = key_for(OWNER, &[], "how do I resize a photo?");
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
    /// Note which direction this protects: it stops a peer READING
    /// another's taint, and it equally stops one peer's clean-looking
    /// conversation being used to launder another's.
    #[test]
    fn one_peers_conversation_is_not_another_peers() {
        let store = TaintStore::default();
        let mine = key_for(":1.42", &[], "summarise this");
        let theirs = key_for(":1.99", &[], "summarise this");
        assert_ne!(mine, theirs);
        let t = store.open(Some(&mine), &[]);
        t.add("mail");
        store.close(Some(&mine), &t);
        assert!(store.open(Some(&theirs), &[]).is_clean());
    }

    /// The taint only ever grows. A later run of the same conversation
    /// cannot hand back a smaller set and quietly narrow what the
    /// conversation has read — the asymmetry `Trigger::resolve` already
    /// uses, one level up.
    #[test]
    fn a_run_can_add_to_its_conversations_taint_and_never_subtract() {
        let store = TaintStore::default();
        let k = key_for(OWNER, &[], "start");
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
        let k = key_for(OWNER, &[], "what is in this?");
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
        let k = key_for(OWNER, &[], "what is 2 + 2?");
        let t = store.open(Some(&k), &[]);
        assert_eq!(chain_of(&t), vec!["user"]);
        store.close(Some(&k), &t);
        // Nothing was read, so nothing is stored — an empty entry per
        // conversation would spend the bound on silence.
        assert!(store.is_empty());
        let k2 = key_for(OWNER, &[user("what is 2 + 2?")], "and 3 + 3?");
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

    /// The bound holds, and it evicts the oldest conversation rather
    /// than the busiest.
    #[test]
    fn the_store_is_bounded() {
        let store = TaintStore::default();
        let first = key_for(OWNER, &[], "conversation 0");
        for i in 0..MAX_CONVERSATIONS + 10 {
            let k = key_for(OWNER, &[], &format!("conversation {i}"));
            let t = Taint::new();
            t.add("web");
            store.close(Some(&k), &t);
        }
        assert_eq!(store.len(), MAX_CONVERSATIONS);
        assert!(
            store.tags_of(&first).is_empty(),
            "the oldest conversation should have been the one evicted"
        );
    }

    /// The key is the first USER turn, not the first turn of any role —
    /// an assistant message is the model's own text, and letting it name
    /// the conversation would put the identity of the taint bucket inside
    /// the thing the taint exists to contain.
    #[test]
    fn the_model_does_not_get_to_name_the_conversation() {
        let a = key_for(OWNER, &[assistant("hello there"), user("the ask")], "next");
        let b = key_for(
            OWNER,
            &[assistant("something else"), user("the ask")],
            "next",
        );
        assert_eq!(a, b, "the model's own words changed the conversation key");
        let c = key_for(OWNER, &[assistant("hello there"), user("other")], "next");
        assert_ne!(a, c);
    }
}
