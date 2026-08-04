//! Verdicts for **Agent Bus tool calls** (issues #251, #252, ADR-0029).
//!
//! The shell side of this crate has had a refused class since ADR-0029:
//! [`crate::Verdict::Deny`] is unoverridable, and `tests/corpus.rs`
//! enumerates the categories it exists for. The Agent Bus honoured none
//! of it — its tiers topped out at *destructive → confirm-modal*. So a
//! model that TYPED `rm -rf /` was refused with no dialog, and a model
//! that CALLED A TOOL doing the same thing got a modal with an Allow
//! button. Same damage, two policies (#251).
//!
//! This module is the second policy deleted. It answers one question —
//! *may this tool call happen* — with the same vocabulary the shell
//! guard already uses, and it is a pure function so it can be tested
//! exhaustively and cannot be talked out of (CLAUDE.md 6a).
//!
//! # The four verdicts (#252)
//!
//! | | Meaning |
//! |---|---|
//! | [`ActionVerdict::HardNo`] | No legitimate agent workflow requires this, ever. Refused; the surface REPORTS, with no approving control. |
//! | [`ActionVerdict::No`] | Out of bounds for the current grant. Refused, naming the scope that would permit it — but nothing in the dialog widens it. |
//! | [`ActionVerdict::Ask`] | In bounds and consequential. Ask, with the effect in plain language. |
//! | [`ActionVerdict::Ask`] with `may_remember` | The dialog may offer "always allow" for this (app, class, scope). |
//!
//! **HARD NO is a property of the ACTION; NO is a property of the
//! CURRENT PERMISSION.** Collapsing them would either make refusals
//! overridable or make ordinary out-of-scope work permanently
//! impossible.
//!
//! # The load-bearing part: the verdict is computed, not declared
//!
//! A tool's manifest tier is a *ceiling the app asked for*, not a
//! verdict. The same call is `Ask` or `No` or `HardNo` depending on
//! where it points, so the answer is a function of `(tool, arguments,
//! grant)`. A tool named `tidy_up` can still target `/`.
//!
//! # Why every string in the arguments is read
//!
//! An argument's NAME is the app's choice, and the app may be the thing
//! we are defending against. Enumerating the key names that carry a path
//! (`path`, `target`, `file`, …) is the same unwinnable denylist
//! `harnessd`'s workspace module already refuses to play; an attacker
//! simply names the key something else. So [`judge`] walks the whole
//! argument tree and judges every string in it that is shaped like a
//! filesystem path. A call is judged by its worst argument.
//!
//! # What this module is NOT
//!
//! It does not decide *how* to confirm — the tier machinery in agentd
//! still does that, and this layer may only refuse or add a description.
//! It never lowers a confirmation. And it is not the kernel: Unix
//! permissions are the first mechanism for another user's files, and the
//! `not-yours` category here is defence in depth for the cases where the
//! kernel does not object (root, shared mounts, world-writable paths).

use std::path::{Component, Path, PathBuf};

/// What a call may do, as the manifest declares it (the *ceiling*).
///
/// Mapped from agentd's `Tier`, which the manifest loader has already
/// raised to at least the floor the tool's own NAME implies (#56) — so a
/// `delete_everything` declaring `read` arrives here as [`Class::Delete`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Read,
    Write,
    Delete,
}

impl Class {
    fn verb(self) -> &'static str {
        match self {
            Class::Read => "read",
            Class::Write => "write to",
            Class::Delete => "delete",
        }
    }

    /// Does this class change anything? Reads are consequential too
    /// (exfiltration needs no write), but only these can destroy.
    fn mutates(self) -> bool {
        self != Class::Read
    }
}

/// What woke the run up (ADR-0036 §1, resolved by harnessd — never
/// claimed by a message).
///
/// The ladder in #252 gives the home content directories to `Prompt`
/// runs only: a person is present and asking. A schedule or an event has
/// untrusted provenance by construction and stays inside the workspace,
/// because exfiltration needs no delete and a delete-confirm is
/// therefore no protection at all for a hostile-provenance run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A person typed this.
    Prompt,
    /// A schedule, an event, an app — anything that is not a person at a
    /// prompt. Fail closed: unknown is Unattended.
    Unattended,
}

/// What this caller has been granted, and who it is acting as.
///
/// Everything here comes from OUTSIDE the model's reach: `$HOME` and the
/// uid from the process, the workspace from a folder a person chose in a
/// file chooser (`harnessd::workspace`), the trigger from the resolved
/// call class. Nothing in a tool call can alter it.
#[derive(Debug, Clone)]
pub struct Grant {
    /// The acting user's home. `None` fails closed: with no home we
    /// cannot tell "yours" from "someone else's", so nothing absolute is
    /// in bounds.
    pub home: Option<PathBuf>,
    /// The acting user's uid, for the not-yours category. `None` skips
    /// the ownership probe (the lexical half still applies).
    pub uid: Option<u32>,
    /// The folder a person chose for this run, if any.
    pub workspace: Option<PathBuf>,
    /// A tree the agent OWNS — nothing of the person's is in it, so a
    /// delete there destroys only the agent's own work.
    ///
    /// **This does not exist yet.** `harnessd` has one notion of a
    /// working folder and it is the owner's data. Always `None` in
    /// production today; the row is here because it is the only row in
    /// #252's ladder where silent deletion is defensible, and leaving it
    /// out would invite someone to grant that property to a workspace.
    pub scratch: Option<PathBuf>,
    /// Hidden paths the OWNER added out of band (#253's Settings page).
    /// Empty today — there is no surface that writes it, and a dialog
    /// must never add to it.
    pub allowlist: Vec<PathBuf>,
    /// Paths the OWNER put OUT of bounds (#253). The mirror of
    /// `allowlist`, and safe from anywhere a dialog can reach: this can
    /// only ever add a refusal. See `crate::protections`.
    pub protections: crate::Protections,
    pub trigger: Trigger,
    /// True when nothing untrusted is in the trigger chain. Only used to
    /// decide whether "always allow" may be OFFERED — never to widen
    /// scope.
    pub trusted_chain: bool,
}

impl Default for Grant {
    /// The empty grant: no home, no workspace, unattended, untrusted.
    /// Every absolute target is out of bounds under it.
    fn default() -> Self {
        Grant {
            home: None,
            uid: None,
            workspace: None,
            scratch: None,
            allowlist: Vec::new(),
            protections: crate::Protections::default(),
            trigger: Trigger::Unattended,
            trusted_chain: false,
        }
    }
}

impl Grant {
    /// The grant a daemon running as this user starts from: its own home
    /// and uid, no workspace, no scratch, nothing allowlisted.
    pub fn for_this_user() -> Grant {
        Grant {
            home: std::env::var_os("HOME").map(PathBuf::from),
            #[cfg(unix)]
            uid: Some(unsafe { libc::getuid() }),
            #[cfg(not(unix))]
            uid: None,
            // The owner's own out-of-bounds folders, read ambiently
            // (#253). HERE and not at each call site, because this is
            // the one constructor every real grant comes from — a
            // protection loaded in some paths and not others is a
            // setting that works until it does not, which is worse than
            // one that never worked.
            //
            // Ambient like `overrides::active()`, and for the same
            // reason: nothing the model emits reaches it. That argument
            // is weaker here than there, because tightening is safe from
            // anywhere — but reading a security file one way in one
            // place and another way elsewhere is how the two dock lists
            // in #239 drifted.
            protections: crate::protections::active(),
            ..Grant::default()
        }
    }

    pub fn with_trigger(mut self, trigger: Trigger) -> Grant {
        self.trigger = trigger;
        self
    }

    pub fn with_trusted_chain(mut self, trusted: bool) -> Grant {
        self.trusted_chain = trusted;
        self
    }

    /// Every spelling of this user's home — as configured and as the
    /// filesystem resolves it. Empty when there is no home, which is the
    /// fail-closed state.
    fn home_roots(&self) -> Vec<PathBuf> {
        roots_of(self.home.as_deref())
    }
}

/// One tool call, as the bus received it.
pub struct Action<'a> {
    pub app_id: &'a str,
    pub tool: &'a str,
    pub class: Class,
    pub args: &'a serde_json::Value,
}

/// What the policy says about one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionVerdict {
    /// Refused because of what it IS. No dialog, no flag, no grant makes
    /// this available to an agent. The owner still has a terminal.
    HardNo { rule: &'static str, reason: String },
    /// Refused because of where it POINTS, given the current grant.
    /// `needs` names the scope that would permit it — as information,
    /// not as a control: nothing in the refusal widens anything.
    No {
        rule: &'static str,
        reason: String,
        needs: String,
    },
    /// In bounds and consequential: ask, with `effect` in plain language.
    ///
    /// `may_remember` is whether the surface may offer "always allow"
    /// for this (app, class, scope). It is never true for an untrusted
    /// chain — "always allow" on something a hostile page suggested is
    /// the failure this whole ladder exists to prevent — and there is no
    /// verdict on which it is true and the call is refused.
    Ask { effect: String, may_remember: bool },
    /// In bounds and unremarkable. This does not bypass the tier
    /// machinery; it declines to add anything to it.
    Allow,
}

impl ActionVerdict {
    /// Is this call refused? Both refusing verdicts, because the one
    /// thing every caller must get right is "do not dispatch".
    pub fn is_refused(&self) -> bool {
        matches!(
            self,
            ActionVerdict::HardNo { .. } | ActionVerdict::No { .. }
        )
    }

    /// Is this the class of refusal no dialog may ever approve?
    pub fn is_hard_no(&self) -> bool {
        matches!(self, ActionVerdict::HardNo { .. })
    }

    pub fn rule(&self) -> Option<&'static str> {
        match self {
            ActionVerdict::HardNo { rule, .. } | ActionVerdict::No { rule, .. } => Some(rule),
            _ => None,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            ActionVerdict::HardNo { reason, .. } | ActionVerdict::No { reason, .. } => Some(reason),
            _ => None,
        }
    }

    fn severity(&self) -> u8 {
        match self {
            ActionVerdict::Allow => 0,
            ActionVerdict::Ask { .. } => 1,
            ActionVerdict::No { .. } => 2,
            ActionVerdict::HardNo { .. } => 3,
        }
    }

    /// The stricter of two verdicts. A call is judged by its worst
    /// argument, exactly as a command line is judged by its worst
    /// segment (`Verdict::worst`).
    fn worst(self, other: Self) -> Self {
        if other.severity() > self.severity() {
            other
        } else {
            self
        }
    }
}

/// Every rule id this module can emit, and what it means.
///
/// Exported so `lisa guard list` can show them without keeping a second
/// copy that drifts (`cli/lisa/src/guard.rs` had exactly that problem
/// for the shell rules and a test to catch it).
pub const BUS_RULES: &[(&str, &str)] = &[
    (
        "exec.shell",
        "a tool that hands the model a shell or runs an arbitrary command",
    ),
    (
        "escalate.privilege",
        "a call that asks to run as root — an agent never escalates",
    ),
    (
        "fill.password_field",
        "typing into a credential field; no agent workflow needs this (#260)",
    ),
    (
        "disk.raw_write",
        "a call whose target is a raw block device",
    ),
    (
        "rm.system_path",
        "a call that would destroy the system or a whole home",
    ),
    (
        "audit.erase",
        "a call that would erase the Ledger, the journal or the logs",
    ),
    (
        "fs.not_yours",
        "a target that resolves to another user's file, wherever it lives",
    ),
    (
        "owner.protected_path",
        "a folder this machine's owner put out of bounds in Settings (#253) — the only refusal here they can remove",
    ),
    (
        "scope.hidden_folder",
        "anything inside ~/.* — where programs keep credentials, and where Lisa keeps the Ledger and grants",
    ),
    (
        "scope.outside_home",
        "a target outside this user's home directory",
    ),
    (
        "scope.unattended_reach",
        "the home content directories, reached by a run no person started",
    ),
];

/// The rule ids that are HARD NO — refused because of what the action
/// is, not because of the current grant.
pub const HARD_NO_RULES: &[&str] = &[
    "exec.shell",
    "escalate.privilege",
    "fill.password_field",
    "disk.raw_write",
    "rm.system_path",
    "audit.erase",
    "fs.not_yours",
    // The owner's own (#253). In the catalogue because a refusal the
    // person configured must be as visible and as explicable as one we
    // shipped — Settings lists both, and only this one is removable.
    "owner.protected_path",
];

/// The verdict for one tool call.
///
/// Order matters and is deliberate: the HARD NO categories are decided
/// first and terminate, because "this is not something Lisa will do" is
/// a stronger and more useful statement than "not with this grant" — and
/// because a refusal must not become an oracle. `/tmp/x` is out of scope
/// whether or not it exists; only paths the agent could otherwise reach
/// are ever probed on disk.
pub fn judge(action: &Action, grant: &Grant) -> ActionVerdict {
    let mut worst = ActionVerdict::Allow;

    // --- HARD NO by what the call IS. No target needed. ---
    for rule in [hands_over_a_shell, escalates_privilege, fills_a_credential] {
        if let Some(v) = rule(action) {
            return v;
        }
    }

    // --- Everything else is decided by where the call POINTS. ---
    let targets = path_arguments(action.args);
    if targets.is_empty() {
        // No path in the arguments: the call acts on the app's own state
        // (a calendar event, a note body). The tier machinery is the
        // authority there; this layer adds a description and nothing more.
        return if action.class.mutates() {
            ActionVerdict::Ask {
                effect: format!("{} in {}", describe_tool(action), action.app_id),
                may_remember: may_remember(grant),
            }
        } else {
            ActionVerdict::Allow
        };
    }

    for raw in targets {
        worst = worst.worst(judge_target(action, grant, &raw));
        if worst.is_hard_no() {
            break; // terminal, and the first reason is the most specific
        }
    }
    worst
}

/// One path argument, judged.
fn judge_target(action: &Action, grant: &Grant, raw: &str) -> ActionVerdict {
    let Some(target) = Target::of(raw, grant) else {
        // A path we cannot place — `$HOME/x` with no home, or a relative
        // name with no granted folder to hang it on. Fail closed.
        return ActionVerdict::No {
            rule: "scope.outside_home",
            reason: format!("`{raw}` cannot be placed relative to anything you granted"),
            needs: "a working folder, chosen by you, that the path sits inside".into(),
        };
    };

    // HARD NO categories that depend on the target, most specific first.
    if let Some(v) = writes_raw_device(action, &target)
        .or_else(|| touches_another_users_file(grant, &target))
        .or_else(|| destroys_the_system_or_a_home(action, grant, &target))
        .or_else(|| erases_the_record(action, grant, &target))
        .or_else(|| the_owner_put_it_out_of_bounds(action, grant, &target))
    // Last of the HARD NOs, deliberately: a built-in reason is more
    // specific and more useful than "you protected this folder", so
    // it wins when both apply. Adding a protection can only ever
    // turn an Allow or an Ask into a refusal — never the reverse.
    {
        return v;
    }

    scope_of(action, grant, &target)
}

/// One path argument in every spelling that matters.
///
/// Both are load-bearing and neither is enough alone:
///
/// - **`lexical`** is the path with `.` and `..` collapsed and nothing
///   else touched. Rules written against `/etc` must be judged here,
///   because canonicalisation can move a path out from under them —
///   `/etc` resolves to `/private/etc` on macOS, and `/home` is a
///   symlink on plenty of systems, so a canonical-only check silently
///   stops matching the very thing it was written for. `harnessd`'s
///   workspace module found this the same way: by a test, not by
///   reading.
/// - **`resolved`** follows symlinks as far as they exist, which is the
///   only way to see that `workspace/shared/notes.txt` is really
///   `/home/alice/notes.txt`. Ownership and containment are properties
///   of the resolved target, not of the string (#251).
///
/// So a target is refused when **either** spelling is out of bounds, and
/// in bounds only when **every** spelling is. That is the fail-closed
/// direction, and it is why these are kept as a set rather than as one
/// "correct" path.
struct Target {
    raw: String,
    forms: Vec<PathBuf>,
}

impl Target {
    fn of(raw: &str, grant: &Grant) -> Option<Target> {
        let absolute = absolutize(raw, grant)?;
        let lexical = lexical_normalize(&absolute);
        let mut forms = vec![lexical.clone()];
        for form in resolution_chain(&lexical) {
            if !forms.contains(&form) {
                forms.push(form);
            }
        }
        Some(Target {
            raw: raw.to_string(),
            forms,
        })
    }

    fn forms(&self) -> impl Iterator<Item = &Path> {
        self.forms.iter().map(PathBuf::as_path)
    }

    /// True when *any* spelling matches — how a refusal is decided.
    fn any(&self, f: impl Fn(&Path) -> bool) -> bool {
        self.forms().any(f)
    }

    /// True when *every* spelling is inside one of `roots` — how being
    /// in bounds is decided. Empty roots is never in bounds.
    fn all_inside(&self, roots: &[PathBuf]) -> bool {
        !roots.is_empty() && self.forms().all(|p| roots.iter().any(|r| p.starts_with(r)))
    }

    /// The resolved spelling, for the ownership probe and for display.
    fn resolved(&self) -> &Path {
        self.forms
            .last()
            .map(PathBuf::as_path)
            .unwrap_or(Path::new(""))
    }
}

/// Every spelling of a granted root, so a target's spellings can be
/// compared against it without one side being canonical and the other not.
fn spellings(p: &Path) -> Vec<PathBuf> {
    let lexical = lexical_normalize(p);
    let real = real_path(&lexical);
    if lexical == real {
        vec![lexical]
    } else {
        vec![lexical, real]
    }
}

fn roots_of(p: Option<&Path>) -> Vec<PathBuf> {
    p.map(spellings).unwrap_or_default()
}

// ---------------------------------------------------------------------
// HARD NO — category by category.
// ---------------------------------------------------------------------

/// **Category 5: handing the model a shell.**
///
/// This one is about the TOOL, not its target, and that is the point: a
/// bus tool that runs a command line puts every entry in this crate's
/// shell corpus back on the table, routed around the reader that was
/// written to stop them. There is no argument that makes it safe, so
/// there is nothing to inspect.
///
/// Detected from the tool's own name and from the argument KEYS — a tool
/// called `tidy_up` taking `{"command": "..."}` is a shell with a
/// friendlier label.
fn hands_over_a_shell(action: &Action) -> Option<ActionVerdict> {
    /// Words that mean "this runs something". `run` alone is not here:
    /// `run_report` is a report. `system` is not here either — it would
    /// refuse every `system_status` read tool on the bus.
    const EXEC_WORDS: &[&str] = &[
        "exec",
        "execute",
        "shell",
        "bash",
        "zsh",
        "ksh",
        "sh",
        "terminal",
        "subprocess",
        "spawn",
        "eval",
        "command",
        "cmd",
        "interpreter",
        "repl",
        "pty",
    ];
    const EXEC_KEYS: &[&str] = &["command", "cmd", "shell", "script", "argv", "exec"];

    let named = words(action.tool)
        .iter()
        .any(|w| EXEC_WORDS.contains(&w.as_str()));
    let keyed = keys_of(action.args)
        .iter()
        .any(|k| EXEC_KEYS.contains(&k.to_ascii_lowercase().as_str()));
    (named || keyed).then(|| ActionVerdict::HardNo {
        rule: "exec.shell",
        reason: format!(
            "`{}` runs a command of the model's choosing. An agent does not get a \
             shell — everything the shell guard refuses would arrive through this one",
            action.tool
        ),
    })
}

/// **Category 3: escalating privilege.**
///
/// Already `Deny` on the shell side (`escalate.privilege`), and the bus
/// is now made to agree — the id is deliberately the same one, so a
/// person reading the Ledger sees one rule rather than two spellings of
/// it. CLAUDE.md 7b puts the same line under the install path: nothing
/// user-facing should need `sudo`.
fn escalates_privilege(action: &Action) -> Option<ActionVerdict> {
    const ESCALATORS: &[&str] = &["sudo", "doas", "pkexec", "run0", "setuid", "escalate"];
    let named = words(action.tool)
        .iter()
        .any(|w| ESCALATORS.contains(&w.as_str()));

    /// One predicate for both spellings of the request — a string
    /// `"as_root": "yes"` and a boolean `"as_root": true` are the same
    /// ask, and having written it twice is how the boolean half was
    /// missing its own key list until a test said so.
    fn asks_for_root(key: &str) -> bool {
        let k = key.to_ascii_lowercase();
        k.contains("sudo") || k.contains("privileg") || k.contains("elevat") || k.contains("root")
    }

    let asked = strings_of(action.args).iter().any(|(key, value)| {
        // A value that IS an escalator, wherever it sits.
        let is_escalator = value
            .split_whitespace()
            .next()
            .map(|first| ESCALATORS.contains(&first.trim_start_matches("/usr/bin/")))
            .unwrap_or(false);
        (asks_for_root(key) && truthy(value)) || is_escalator
    }) || flags_of(action.args)
        .iter()
        .any(|(key, on)| *on && asks_for_root(key));

    (named || asked).then(|| ActionVerdict::HardNo {
        rule: "escalate.privilege",
        reason: format!(
            "`{}` asks to act as root. An agent never escalates privilege on its \
             own — that is a decision for the person at their own prompt",
            action.tool
        ),
    })
}

/// **Category 7: a password field is never a valid `fill` target (#260).**
///
/// There is no legitimate agent workflow that types a credential. A
/// person who wants to log in does it themselves.
///
/// Honest limit, and it is the reason #260 asks the browser for the
/// other half: this sees the SELECTOR, not the field. #212 landed
/// `fill(selector:"#q")` in a field named `password` because the page
/// owned the JS world — no string rule here would have caught it. The
/// argument half is what a deterministic guard can do; refusing a field
/// the browser has RESOLVED as `type=password` belongs in Surfer.
fn fills_a_credential(action: &Action) -> Option<ActionVerdict> {
    const FILL_WORDS: &[&str] = &["fill", "autofill", "type", "keyboard", "input", "enter"];
    const CREDENTIAL: &[&str] = &[
        "password",
        "passwd",
        "passphrase",
        "otp",
        "totp",
        "cvv",
        "cvc",
        "credential",
        "secret",
    ];
    if !action.class.mutates() {
        return None;
    }
    if !words(action.tool)
        .iter()
        .any(|w| FILL_WORDS.contains(&w.as_str()))
    {
        return None;
    }
    let hit = strings_of(action.args).iter().any(|(key, value)| {
        let haystack = format!("{key} {value}").to_ascii_lowercase();
        CREDENTIAL.iter().any(|c| haystack.contains(c))
    });
    hit.then(|| ActionVerdict::HardNo {
        rule: "fill.password_field",
        reason: format!(
            "`{}` is aimed at a credential field. Nothing an agent legitimately \
             does involves typing a password — sign in yourself",
            action.tool
        ),
    })
}

/// **Category 2: writing raw devices.**
fn writes_raw_device(action: &Action, target: &Target) -> Option<ActionVerdict> {
    (action.class.mutates() && target.any(|p| starts_with_str(p, "/dev"))).then(|| {
        ActionVerdict::HardNo {
            rule: "disk.raw_write",
            reason: format!(
                "`{}` is a raw device — writing it destroys whatever is on the disk",
                target.raw
            ),
        }
    })
}

/// **Category 1: destroying the system or a whole home.**
///
/// The two predicates are the shell guard's own — the *same* functions
/// `rm -rf /etc` is refused by, not a second list that agrees today
/// (#251: one vocabulary, ADR-0050's argument for a single authority).
fn destroys_the_system_or_a_home(
    action: &Action,
    grant: &Grant,
    target: &Target,
) -> Option<ActionVerdict> {
    if !action.class.mutates() {
        return None;
    }
    let homes = grant.home_roots();
    let hit = target.any(|p| {
        let t = p.to_string_lossy().to_string();
        homes.iter().any(|h| h == p)
            || crate::rules::is_system_target(&t)
            || crate::rules::is_under_system_root(&t)
    });
    hit.then(|| ActionVerdict::HardNo {
        rule: "rm.system_path",
        reason: format!(
            "`{}` is the system, or a whole home directory. There is no agent task \
             that needs it {}",
            target.raw,
            match action.class {
                Class::Delete => "deleted",
                _ => "rewritten",
            }
        ),
    })
}

/// **The owner's own refusals (#253).**
///
/// Checked as an ADDITIONAL refusal, never as a lookup that could
/// answer "allowed": a path this set does not cover means the set has
/// no opinion, so control falls through to every built-in exactly as
/// before. That is what makes the Settings page safe to reach from a
/// dialog — the worst a hostile write here can do is refuse too much.
///
/// Read-tier calls are exempt on purpose. The owner protected the
/// folder from being CHANGED; refusing to let a summary mention it
/// would make the setting a censor rather than a guard, and people who
/// find a protection too blunt turn it off.
fn the_owner_put_it_out_of_bounds(
    action: &Action,
    grant: &Grant,
    target: &Target,
) -> Option<ActionVerdict> {
    if !action.class.mutates() || grant.protections.is_empty() {
        return None;
    }
    let hit = target.any(|p| grant.protections.covers(p));
    hit.then(|| ActionVerdict::HardNo {
        rule: "owner.protected_path",
        reason: format!(
            "`{}` is inside a folder you put out of bounds in Settings. \
             Remove it there if you want agents to act on it.",
            target.raw
        ),
    })
}

/// **Category 4: erasing the record.**
///
/// The Ledger is append-only precisely so the trail survives; anything
/// that can erase it can erase the evidence of what else it did. Lisa's
/// own state lives in `~/.local/share/lisa/` — `ledger.db` beside
/// `grants.db`, which is also why no dialog may ever allowlist a hidden
/// folder (#252): one "always allow" there and an agent edits its own
/// permissions.
fn erases_the_record(action: &Action, grant: &Grant, target: &Target) -> Option<ActionVerdict> {
    if !action.class.mutates() {
        return None;
    }
    let state: Vec<PathBuf> = grant
        .home_roots()
        .iter()
        .map(|h| h.join(".local/share/lisa"))
        .collect();
    let hit = target.any(|p| {
        let t = p.to_string_lossy().to_string();
        state.iter().any(|s| p.starts_with(s))
            || t.starts_with("/var/log")
            || t.ends_with(".ledger")
            || t.ends_with("_history")
            || t.ends_with("ledger.db")
    });
    hit.then(|| ActionVerdict::HardNo {
        rule: "audit.erase",
        reason: format!(
            "`{}` is part of the record of what happened. The Ledger is append-only \
             so the trail survives whatever else went wrong",
            target.raw
        ),
    })
}

/// **Category 6: another user's files, wherever they live.**
///
/// ADR-0029's second test asks whether a guardrail is aimed at the model
/// or at the owner — and "the owner" is ambiguous the moment there is
/// more than one user. Being root does not make Alice's notes yours to
/// delete through an agent.
///
/// **Unix permissions are the first mechanism and get the credit**:
/// agentd runs as one user, so an agent acting as `lisa` cannot unlink
/// `/home/alice/notes.txt` — it lacks permission and no policy is
/// consulted. This rule is defence in depth for where that protection
/// does not apply: elevated contexts, shared group directories, network
/// mounts, and world-writable locations. A catalogue that presented
/// itself as *the* protection would be the "documented guarantee not in
/// force" defect this repo keeps finding.
///
/// Two halves, because neither alone is enough:
///
/// - **Lexical**, on the resolved path: `/home/<someone-else>`,
///   `/Users/<someone-else>`, `/root`. Works on paths that do not exist
///   yet, which the stat cannot.
/// - **Ownership**, by `stat` on the nearest existing ancestor, and only
///   for paths the agent could otherwise reach. Outside the home
///   everything is already refused, so probing there would buy nothing
///   and would turn the refusal into an existence oracle (ADR-0033).
fn touches_another_users_file(grant: &Grant, target: &Target) -> Option<ActionVerdict> {
    let refuse = || ActionVerdict::HardNo {
        rule: "fs.not_yours",
        reason: format!(
            "`{}` resolves to a file that is not this user's. Another person's files \
             are not yours to touch through an agent, whoever you are",
            target.raw
        ),
    };
    let homes = grant.home_roots();
    if target.any(|p| lexically_another_users_home(p, &homes)) {
        return Some(refuse());
    }
    // Only probe inside what this grant could reach: see the doc above.
    let reachable: Vec<PathBuf> = [
        grant.home.as_deref(),
        grant.workspace.as_deref(),
        grant.scratch.as_deref(),
    ]
    .into_iter()
    .flat_map(roots_of)
    .collect();
    if !target.all_inside(&reachable) {
        return None;
    }
    let uid = grant.uid?;
    (owner_uid(target.resolved())? != uid).then(refuse)
}

/// `/home/<name>` (or `/Users/<name>`, `/var/home/<name>`) that is not
/// this user's own home, and `/root` when this user is not root.
fn lexically_another_users_home(p: &Path, homes: &[PathBuf]) -> bool {
    let t = p.to_string_lossy().to_string();
    let ours = |theirs: &Path| homes.iter().any(|h| h == theirs);
    if t == "/root" || t.starts_with("/root/") {
        return !ours(Path::new("/root"));
    }
    for root in ["/home/", "/Users/", "/var/home/"] {
        let Some(rest) = t.strip_prefix(root) else {
            continue;
        };
        let Some(who) = rest.split('/').next().filter(|s| !s.is_empty()) else {
            continue;
        };
        return !ours(&PathBuf::from(format!("{root}{who}")));
    }
    false
}

// ---------------------------------------------------------------------
// NO / Ask / Allow — the scope ladder (#252).
// ---------------------------------------------------------------------

/// Where a target sits on the ladder, once it has survived HARD NO.
fn scope_of(action: &Action, grant: &Grant, target: &Target) -> ActionVerdict {
    let raw = &target.raw;
    let homes = grant.home_roots();
    if homes.is_empty() {
        // Fail closed: with no home we cannot tell yours from anyone
        // else's, and a grant whose only real check has evaporated is
        // not a grant.
        return ActionVerdict::No {
            rule: "scope.outside_home",
            reason: format!(
                "`{raw}` cannot be checked: this process cannot tell where your home is"
            ),
            needs: "a session with HOME set".into(),
        };
    }

    // Agent scratch: the one row where silent action is defensible,
    // because the agent created everything in it. Not built yet.
    if target.all_inside(&roots_of(grant.scratch.as_deref())) {
        return ActionVerdict::Allow;
    }

    if !target.all_inside(&homes) {
        return ActionVerdict::No {
            rule: "scope.outside_home",
            reason: format!("`{raw}` is outside your home directory"),
            needs: "a folder inside your home, granted as this run's working folder".into(),
        };
    }

    // Hidden folders are not user content (#231, #252). Structural, not
    // a denylist: a leading dot is the convention by which a program
    // says "this is mine, not the user's work". The person who designed
    // this policy listed four content directories and forgot `~/.*`;
    // anyone enumerating credential stores by hand will miss one.
    let allowed: Vec<PathBuf> = grant.allowlist.iter().flat_map(|a| spellings(a)).collect();
    let hidden = target.any(|p| {
        homes
            .iter()
            .filter_map(|h| p.strip_prefix(h).ok())
            .any(hides_something)
    });
    if hidden && !target.all_inside(&allowed) {
        return ActionVerdict::No {
            rule: "scope.hidden_folder",
            reason: format!(
                "`{raw}` is inside a hidden folder — that is where programs keep \
                 configuration and credentials, not where you keep your work"
            ),
            // Named as information, never offered as a control: the
            // allowlist is added by the owner in Settings (#253), out of
            // band, because `~/.local/share/lisa/` holds the Ledger and
            // the grants themselves.
            needs: "an entry you add yourself in Settings › Policies".into(),
        };
    }

    let in_workspace = target.all_inside(&roots_of(grant.workspace.as_deref()));

    // The home content directories are reachable only when a person
    // started the run. Exfiltration needs no delete, so "confirm before
    // deleting" is no protection for a run a hostile page woke up.
    if !in_workspace && grant.trigger != Trigger::Prompt {
        return ActionVerdict::No {
            rule: "scope.unattended_reach",
            reason: format!(
                "`{raw}` is outside this run's working folder, and no person \
                 started this run"
            ),
            needs: "the folder granted as this run's working folder".into(),
        };
    }

    if !action.class.mutates() {
        return ActionVerdict::Allow;
    }
    ActionVerdict::Ask {
        effect: format!(
            "{} `{}`{}",
            action.class.verb(),
            display_rel(&homes, target.resolved()),
            if in_workspace {
                " in this run's working folder"
            } else {
                " in your home"
            }
        ),
        may_remember: may_remember(grant),
    }
}

/// "Always allow" may only be offered where a person is present and
/// nothing untrusted steered the call (#252). It is never offered on a
/// refusal at all — there is no such verdict.
fn may_remember(grant: &Grant) -> bool {
    grant.trusted_chain && grant.trigger == Trigger::Prompt
}

// ---------------------------------------------------------------------
// Reading the arguments.
// ---------------------------------------------------------------------

/// Every string in the argument tree, as `(key, value)`. The key is the
/// nearest enclosing object key, or `""` inside a bare array.
fn strings_of(args: &serde_json::Value) -> Vec<(String, String)> {
    fn walk(v: &serde_json::Value, key: &str, out: &mut Vec<(String, String)>) {
        match v {
            serde_json::Value::String(s) => out.push((key.to_string(), s.clone())),
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, key, out)),
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    walk(v, k, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(args, "", &mut out);
    out
}

/// Every boolean in the argument tree, as `(key, value)`.
fn flags_of(args: &serde_json::Value) -> Vec<(String, bool)> {
    fn walk(v: &serde_json::Value, key: &str, out: &mut Vec<(String, bool)>) {
        match v {
            serde_json::Value::Bool(b) => out.push((key.to_string(), *b)),
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, key, out)),
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    walk(v, k, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(args, "", &mut out);
    out
}

/// Every object key anywhere in the argument tree.
fn keys_of(args: &serde_json::Value) -> Vec<String> {
    fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    out.push(k.clone());
                    walk(v, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(args, &mut out);
    out
}

/// The strings in the arguments that are shaped like filesystem paths.
///
/// Deliberately narrow on prose and wide on paths: a sentence in a note
/// body must not be read as a target (the shell corpus' `MUST_ALLOW`
/// lesson — a guard that refuses ordinary work is one people switch
/// off), while every spelling that names a place must be.
fn path_arguments(args: &serde_json::Value) -> Vec<String> {
    strings_of(args)
        .into_iter()
        .map(|(_, v)| v)
        .filter(|s| is_path_shaped(s))
        .collect()
}

fn is_path_shaped(s: &str) -> bool {
    let t = strip_file_uri(s.trim());
    if t.is_empty() {
        return false;
    }
    if matches!(t, "~" | "$HOME" | "${HOME}" | "..") {
        return true;
    }
    t.starts_with('/')
        || t.starts_with("~/")
        || t.starts_with("$HOME/")
        || t.starts_with("${HOME}/")
        || t.starts_with("./")
        || t.starts_with("../")
        // A traversal segment anywhere is a path claim however it is
        // spelled: `docs/../../../etc/passwd`.
        || t.split('/').any(|c| c == "..")
}

/// A raw argument → an absolute path, before any resolution.
///
/// `~`, `$HOME` and `${HOME}` are expanded because they are how a path
/// inside the home is actually written; a relative name lands wherever
/// the calling app's own jail root is, and the best we know of that is
/// the folder that was granted. With nothing to hang it on, `None` —
/// and an unplaceable path fails closed.
/// `file:///etc/passwd` is a path with a prefix on it. An app that takes
/// URIs rather than paths — a viewer, a browser, anything that speaks
/// `Gio.File` — would otherwise route straight past every rule here.
fn strip_file_uri(s: &str) -> &str {
    s.strip_prefix("file://").map_or(s, |rest| {
        // `file:///etc` → `/etc`; `file://localhost/etc` is the other
        // legal spelling and its authority is not a path component.
        rest.strip_prefix("localhost").unwrap_or(rest)
    })
}

fn absolutize(raw: &str, grant: &Grant) -> Option<PathBuf> {
    let t = strip_file_uri(raw.trim().trim_matches(['"', '\'']));
    let home = grant.home.as_deref();
    if let Some(rest) = t
        .strip_prefix("~/")
        .or_else(|| t.strip_prefix("$HOME/"))
        .or_else(|| t.strip_prefix("${HOME}/"))
    {
        return Some(home?.join(rest));
    }
    if matches!(t, "~" | "$HOME" | "${HOME}") {
        return Some(home?.to_path_buf());
    }
    if t.starts_with('/') {
        return Some(PathBuf::from(t));
    }
    Some(
        grant
            .workspace
            .as_deref()
            .or(grant.scratch.as_deref())
            .or(home)?
            .join(t),
    )
}

/// Collapse `.` and `..` without touching the filesystem, so a path that
/// does not exist is still judged by where it points.
fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolve `p` component by component, following every symlink that
/// exists, and keeping whatever does not exist verbatim.
///
/// Deliberately **not** [`std::fs::canonicalize`]. Canonicalize fails
/// outright when the leaf is missing — which is the ordinary case for a
/// write — and, worse, it reports a *dangling* symlink as merely absent.
/// `path.rs` already had to learn that: a link pointing at a target that
/// does not exist yet is still followed on write, so treating it as an
/// absent file is how a write lands outside the root. A symlink named
/// `workspace/shared` → `/home/alice` on a machine with no `/home/alice`
/// is exactly that shape, and canonicalize would have called the target
/// contained.
///
/// Bounded at 40 links: a symlink cycle is otherwise an infinite loop
/// inside a guard, which is a denial of service in the one place that
/// must always answer.
///
/// Returns **every** path the resolution passes through, not only the
/// last one. `/home/alice/notes.txt` may itself resolve onward — on
/// macOS `/home` is under an automounter, on many systems it is a
/// symlink into another volume — and the rule that recognises another
/// user's home is written against the spelling `/home/…`, which only
/// exists in the middle of the chain. Keeping the chain is what stops
/// this being a machine-specific list of mount points.
fn resolution_chain(p: &Path) -> Vec<PathBuf> {
    const MAX_HOPS: usize = 40;
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut hops = 0usize;
    let mut out = PathBuf::new();
    let mut queue: std::collections::VecDeque<std::ffi::OsString> = p
        .components()
        .map(|c| c.as_os_str().to_os_string())
        .collect();
    while let Some(name) = queue.pop_front() {
        match name.to_string_lossy().as_ref() {
            "/" => {
                out = PathBuf::from("/");
                continue;
            }
            "." => continue,
            ".." => {
                out.pop();
                continue;
            }
            _ => {}
        }
        out.push(&name);
        if hops >= MAX_HOPS {
            continue;
        }
        let is_link = std::fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink());
        if is_link && let Ok(link) = std::fs::read_link(&out) {
            hops += 1;
            out.pop();
            for c in link.components().rev() {
                queue.push_front(c.as_os_str().to_os_string());
            }
            // The spelling this hop produced, tail included.
            let mut snapshot = out.clone();
            for c in &queue {
                if c.to_string_lossy() == "/" {
                    snapshot = PathBuf::from("/");
                } else {
                    snapshot.push(c);
                }
            }
            chain.push(snapshot);
        }
    }
    chain.push(out);
    chain
}

/// The end of the chain: `p` with every symlink followed.
fn real_path(p: &Path) -> PathBuf {
    resolution_chain(p).pop().unwrap_or_else(|| p.to_path_buf())
}

/// The owning uid of `p`, or of the nearest ancestor that exists.
#[cfg(unix)]
fn owner_uid(p: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    let mut cur = Some(p);
    while let Some(path) = cur {
        if let Ok(md) = std::fs::symlink_metadata(path) {
            return Some(md.uid());
        }
        cur = path.parent();
    }
    None
}

#[cfg(not(unix))]
fn owner_uid(_p: &Path) -> Option<u32> {
    None
}

fn hides_something(rel: &Path) -> bool {
    rel.components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
}

fn starts_with_str(p: &Path, prefix: &str) -> bool {
    let t = p.to_string_lossy().to_string();
    t == prefix || t.starts_with(&format!("{prefix}/"))
}

fn display_rel(homes: &[PathBuf], p: &Path) -> String {
    homes
        .iter()
        .find_map(|h| p.strip_prefix(h).ok())
        .map(|rel| format!("~/{}", rel.display()))
        .unwrap_or_else(|| p.display().to_string())
}

/// A tool name as whole words, lowercased — the same splitting agentd's
/// `implied_floor` uses, so `set` fires on `set_alarm` and not on
/// `get_settings`.
fn words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "root"
    )
}

/// The tool's own name, made readable for a dialog: `delete_event` →
/// "delete event". Not a description of the EFFECT — the app's
/// description would be, and the bus adds it — but better than a raw
/// identifier, which is what #251's screenshot complained about.
fn describe_tool(action: &Action) -> String {
    words(action.tool).join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A home this test process genuinely owns, with a person at the
    /// prompt and nothing untrusted in the chain — the most permissive
    /// grant there is, so anything refused under it is refused on its
    /// own merits and not for want of a permission.
    fn grant_at(home: &Path) -> Grant {
        let real = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
        Grant {
            uid: owner_uid(&real),
            home: Some(real),
            trigger: Trigger::Prompt,
            trusted_chain: true,
            ..Grant::default()
        }
    }

    fn act<'a>(tool: &'a str, class: Class, args: &'a serde_json::Value) -> Action<'a> {
        Action {
            app_id: "app.lisaos.Probe244",
            tool,
            class,
            args,
        }
    }

    /// #251's screenshot, as a test. A tool call targeting `/` reached a
    /// modal with an Allow button because its tier said `destructive`
    /// and nothing looked at `{"target":"/"}`.
    #[test]
    fn deleting_everything_in_slash_is_refused_not_confirmed() {
        let home = tempfile::tempdir().unwrap();
        let args = json!({"target": "/"});
        let v = judge(
            &act("delete_everything", Class::Delete, &args),
            &grant_at(home.path()),
        );
        assert!(v.is_hard_no(), "got {v:?}");
        assert_eq!(v.rule(), Some("rm.system_path"));
    }

    /// **The test that matters most (#252):** the same tool and the same
    /// class yield different verdicts for different targets. That is what
    /// proves the verdict is computed rather than read off the manifest.
    #[test]
    fn the_same_tool_call_lands_differently_depending_where_it_points() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(home.join("dev/LandingPage")).unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        let mut grant = grant_at(&home);
        grant.workspace = Some(home.join("dev/LandingPage"));
        let inside = home.join("dev/LandingPage/index.html");
        let elsewhere = home.join("Documents/notes.md");

        let cases: Vec<(serde_json::Value, Option<&str>)> = vec![
            (json!({"path": inside.to_str().unwrap()}), None),
            (json!({"path": elsewhere.to_str().unwrap()}), None),
            (json!({"path": "/"}), Some("rm.system_path")),
            (json!({"path": "/etc/passwd"}), Some("rm.system_path")),
            (json!({"path": "/dev/sda"}), Some("disk.raw_write")),
            (
                json!({"path": "~/.ssh/id_rsa"}),
                Some("scope.hidden_folder"),
            ),
            (json!({"path": "/tmp/scratch"}), Some("scope.outside_home")),
            (
                json!({"path": "/home/alice/notes.txt"}),
                Some("fs.not_yours"),
            ),
        ];
        for (args, expected) in cases {
            let v = judge(&act("delete_file", Class::Delete, &args), &grant);
            assert_eq!(v.rule(), expected, "for {args} the verdict was {v:?}");
            if expected.is_none() {
                assert!(
                    matches!(v, ActionVerdict::Ask { .. }),
                    "an in-bounds delete must ask, not proceed: {v:?}"
                );
            }
        }
    }

    #[test]
    fn a_shell_by_any_name_is_refused() {
        let home = tempfile::tempdir().unwrap();
        for (tool, args) in [
            ("run_command", json!({"line": "ls"})),
            ("exec", json!({"line": "ls"})),
            ("tidy_up", json!({"command": "rm -rf /"})),
            ("helper", json!({"argv": ["sh", "-c", "id"]})),
        ] {
            let v = judge(&act(tool, Class::Write, &args), &grant_at(home.path()));
            assert_eq!(v.rule(), Some("exec.shell"), "{tool} → {v:?}");
        }
    }

    /// A read tool whose name merely mentions the system is not a shell.
    #[test]
    fn ordinary_tools_are_not_mistaken_for_a_shell() {
        let home = tempfile::tempdir().unwrap();
        for tool in ["system_status", "run_report", "list_notes", "read_page"] {
            let args = json!({});
            let v = judge(&act(tool, Class::Read, &args), &grant_at(home.path()));
            assert_eq!(v, ActionVerdict::Allow, "{tool} → {v:?}");
        }
    }

    #[test]
    fn escalation_is_refused_on_the_bus_exactly_as_on_the_shell() {
        let home = tempfile::tempdir().unwrap();
        for (tool, args) in [
            ("install_package", json!({"as_root": true})),
            ("write_file", json!({"privileged": "yes"})),
            ("open", json!({"with": "pkexec gedit"})),
        ] {
            let v = judge(&act(tool, Class::Write, &args), &grant_at(home.path()));
            assert_eq!(v.rule(), Some("escalate.privilege"), "{tool} → {v:?}");
        }
    }

    #[test]
    fn a_credential_field_is_never_a_fill_target() {
        let home = tempfile::tempdir().unwrap();
        for args in [
            json!({"selector": "#password", "value": "hunter2"}),
            json!({"selector": "input[type=password]", "value": "x"}),
            json!({"selector": "[name='passwd']", "value": "x"}),
            json!({"field": "otp", "value": "123456"}),
        ] {
            let v = judge(&act("fill", Class::Write, &args), &grant_at(home.path()));
            assert_eq!(v.rule(), Some("fill.password_field"), "{args} → {v:?}");
        }
        // An ordinary field still fills — the guard sits between the
        // model and the machine, not between a person and their work.
        let ok = json!({"selector": "#search", "value": "lisa os"});
        assert!(!judge(&act("fill", Class::Write, &ok), &grant_at(home.path())).is_refused());
    }

    #[test]
    fn the_ledger_cannot_be_erased_through_a_tool_call() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        for target in [
            "~/.local/share/lisa/ledger.db",
            "~/.local/share/lisa/grants.db",
            "/var/log/lisa.ledger",
        ] {
            let args = json!({"path": target});
            let v = judge(&act("delete_file", Class::Delete, &args), &grant_at(&home));
            assert_eq!(v.rule(), Some("audit.erase"), "{target} → {v:?}");
        }
    }

    /// Ownership is a property of the RESOLVED target: a symlink inside
    /// the workspace pointing at another user's tree is that user's file.
    #[test]
    fn another_users_files_are_refused_however_the_path_is_spelled() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let ws = home.join("dev/app");
        std::fs::create_dir_all(&ws).unwrap();
        let mut grant = grant_at(&home);
        grant.workspace = Some(ws.clone());

        // Lexical: another user's home, wherever it is spelled from.
        for raw in [
            "/home/alice/notes.txt",
            "/home/alice/../alice/notes.txt",
            "/root/.bashrc",
        ] {
            let args = json!({"path": raw});
            let v = judge(&act("read_file", Class::Read, &args), &grant);
            assert_eq!(v.rule(), Some("fs.not_yours"), "{raw} → {v:?}");
        }

        // Resolved: a symlink in the workspace pointing at Alice's tree.
        // The string names the workspace; the target is Alice's.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/home/alice", ws.join("shared")).unwrap();
            let args = json!({"path": ws.join("shared/notes.txt").to_str().unwrap()});
            let v = judge(&act("read_file", Class::Read, &args), &grant);
            assert_eq!(v.rule(), Some("fs.not_yours"), "through a symlink → {v:?}");
        }

        // `..` traversal that lands outside the acting user's tree.
        let up = json!({"path": "../../../../home/alice/notes.txt"});
        let v = judge(&act("read_file", Class::Read, &up), &grant);
        assert!(v.is_refused(), "traversal out of the tree → {v:?}");

        // Ownership: a real file inside the reachable tree whose uid is
        // not the acting user's. The uid is a value this test picks,
        // which is how it runs without root.
        let theirs = ws.join("shared-notes.txt");
        std::fs::write(&theirs, "not ours").unwrap();
        let args = json!({"path": theirs.to_str().unwrap()});
        let someone_else = Grant {
            uid: Some(owner_uid(&theirs).unwrap().wrapping_add(1)),
            ..grant.clone()
        };
        let v = judge(&act("delete_file", Class::Delete, &args), &someone_else);
        assert_eq!(v.rule(), Some("fs.not_yours"), "by uid → {v:?}");
        // …and the same file IS the acting user's under their own uid.
        assert!(!judge(&act("delete_file", Class::Delete, &args), &grant).is_refused());
    }

    /// The refusal must not reveal what exists (ADR-0033). A not-yours
    /// refusal says exactly the same thing whether the file is there or
    /// not — the only difference is the caller's own argument, echoed.
    #[test]
    fn a_not_yours_refusal_reveals_nothing_the_caller_did_not_supply() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let grant = grant_at(&home);
        let (there, gone) = ("/home/alice/notes.txt", "/home/alice/nothing-here.txt");
        let a = judge(
            &act("read_file", Class::Read, &json!({"path": there})),
            &grant,
        );
        let b = judge(
            &act("read_file", Class::Read, &json!({"path": gone})),
            &grant,
        );
        assert_eq!(a.rule(), b.rule());
        assert_eq!(
            a.reason().unwrap().replace(there, "<arg>"),
            b.reason().unwrap().replace(gone, "<arg>"),
            "the refusal differs by more than the caller's own argument"
        );
    }

    /// The trigger class decides the reach (#252). A schedule gets the
    /// workspace; a person at a prompt gets the home content directories.
    #[test]
    fn an_unattended_run_does_not_reach_the_home_content_directories() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(home.join("dev/app")).unwrap();
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        let mut grant = grant_at(&home);
        grant.workspace = Some(home.join("dev/app"));

        let doc = json!({"path": home.join("Documents/notes.md").to_str().unwrap()});
        let ws = json!({"path": home.join("dev/app/index.html").to_str().unwrap()});

        let prompt = grant.clone().with_trigger(Trigger::Prompt);
        assert!(!judge(&act("read_file", Class::Read, &doc), &prompt).is_refused());

        let cron = grant
            .with_trigger(Trigger::Unattended)
            .with_trusted_chain(false);
        let v = judge(&act("read_file", Class::Read, &doc), &cron);
        assert_eq!(v.rule(), Some("scope.unattended_reach"), "{v:?}");
        // …and the workspace still works for it: this narrows reach, it
        // does not stop scheduled work.
        assert!(!judge(&act("read_file", Class::Read, &ws), &cron).is_refused());
    }

    /// "Always allow" is never offered where nothing legitimises it.
    #[test]
    fn always_allow_is_never_offered_to_an_untrusted_run() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(home.join("dev/app")).unwrap();
        let mut grant = grant_at(&home);
        grant.workspace = Some(home.join("dev/app"));
        let args = json!({"path": home.join("dev/app/index.html").to_str().unwrap()});

        let trusted = judge(&act("write_file", Class::Write, &args), &grant);
        assert_eq!(
            trusted,
            ActionVerdict::Ask {
                effect: "write to `~/dev/app/index.html` in this run's working folder".into(),
                may_remember: true
            }
        );

        let tainted = grant.clone().with_trusted_chain(false);
        match judge(&act("write_file", Class::Write, &args), &tainted) {
            ActionVerdict::Ask { may_remember, .. } => assert!(
                !may_remember,
                "a call steered by untrusted content must not be rememberable"
            ),
            other => panic!("expected an ask, got {other:?}"),
        }
        // And no refusal carries the offer at all — there is no verdict
        // on which it could.
        let bad = json!({"path": "/"});
        assert!(matches!(
            judge(&act("write_file", Class::Write, &bad), &grant),
            ActionVerdict::HardNo { .. }
        ));
    }

    /// Fail closed: with no home, nothing absolute is in bounds.
    #[test]
    fn without_a_home_nothing_absolute_is_granted() {
        let args = json!({"path": "/anywhere/at/all"});
        let v = judge(&act("read_file", Class::Read, &args), &Grant::default());
        assert!(v.is_refused(), "{v:?}");
    }

    /// A call with no path in it is the app's own state, and the tier
    /// machinery is the authority there. This layer adds a description.
    #[test]
    fn a_call_with_no_target_is_left_to_the_tier_machinery() {
        let home = tempfile::tempdir().unwrap();
        let g = grant_at(home.path());
        let args = json!({"title": "Dentist", "start": "2026-08-05T09:00"});
        assert!(matches!(
            judge(&act("add_event", Class::Write, &args), &g),
            ActionVerdict::Ask { .. }
        ));
        let none = json!({});
        assert_eq!(
            judge(&act("list_events", Class::Read, &none), &g),
            ActionVerdict::Allow
        );
    }

    /// Prose is not a path. A guard that refuses a note whose text
    /// mentions `/etc` is a guard people route around.
    #[test]
    fn a_sentence_that_mentions_a_path_is_not_a_target() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let g = grant_at(&home);
        for body in [
            "remember to check /etc/fstab tomorrow",
            "guard: rm -rf /etc gets through",
            "https://example.com/docs/index.html",
        ] {
            let args = json!({"title": "note", "body": body});
            let v = judge(&act("create_note", Class::Write, &args), &g);
            assert!(!v.is_refused(), "`{body}` was read as a target: {v:?}");
        }
    }

    #[test]
    fn every_rule_the_module_emits_is_in_its_own_catalogue() {
        for id in HARD_NO_RULES {
            assert!(
                BUS_RULES.iter().any(|(r, _)| r == id),
                "`{id}` is a hard-no with no catalogue entry"
            );
        }
    }
}
