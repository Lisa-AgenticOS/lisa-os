#!/usr/bin/env python3
"""Every shipped unit that runs a Lisa daemon declares an egress posture (#275).

CLAUDE.md rule 5: `lisa-inferenced`, `lisa-contextd` and `lisa-agentd`
never get network access; **only `lisa-remoted`** does — it is the sole
egress broker. That is architecture, so it has to be a mechanism.

What went wrong on 2026-08-05, and why this file exists:

  * `os/packages/lisa/lisa-inferenced-dbus.service` — the per-user
    companion every Assistant and overlay prompt goes through — shipped
    with **no egress sandbox at all**, while two comments inside it
    described one. Rule 5 was prose, on the daemon that handles every
    prompt.
  * The one thing CI did check was a hand-typed loop over six unit
    paths. A seventh unit, or a rename, is invisible to a list nobody
    updates. The `egress` job's own comment admitted this.

So the population is *discovered*, never typed:

  1. Read the installers (`PKGBUILD` install lines into `systemd/system/`
     or `systemd/user/`, plus mkosi's baked-in unit trees). Where the
     build puts a unit is the ground truth about what ships — the same
     reasoning check-user-units.py arrived at, for the same reason: a
     unit's own `[Install]` section lies by omission for timer- and
     D-Bus-activated services.
  2. For each unit take the `ExecStart=` binary. If it is a Lisa binary
     (`lisa`, `lisa-*`, `xdg-desktop-portal-lisa`) it MUST have a
     posture below. **An unknown Lisa binary fails this check** — that
     is the whole design. A fourth no-egress daemon added next year
     cannot slip in unclassified, and a second unit for an already-known
     daemon (which is exactly what lisa-inferenced-dbus was) is picked
     up with no edit here at all.
  3. Units that run something else — mkosi's boot shell scripts, stock
     binaries — are out of rule 5's scope and are skipped by name in
     `--explain` output rather than silently.

The posture is keyed by BINARY, not by unit filename, because the
property is a property of the daemon: lisa-contextd may not reach the
network in a system unit, a user unit, or any unit written after this
comment.

WHAT THE FIRST VERSION OF THAT DISCOVERY MISSED (#291, #292), because
"discovered, not typed" is only as good as the discovery:

  * It matched literal whitespace TOKENS on install lines. A source
    held in a shell variable, produced by a `for` loop, or written as a
    glob resolved to nothing and was **dropped in silence** — and the
    loop form is not hypothetical, PKGBUILD:262 already ships two
    drop-ins that way. Over the unmodified tree, 16 install lines into
    systemd directories resolved to nothing and nobody was told.
  * Its mkosi glob was `usr/lib/systemd/*/*.service`, so a unit baked
    into `mkosi.extra/etc/systemd/system/` was invisible — a directory
    the image build already uses.
  * `*.service.d/*.conf` drop-ins were never read at all, so a drop-in
    handing the network back to a hardened daemon passed both this gate
    and the runtime one.

So the installers are now INTERPRETED rather than pattern-matched: a
small shell walker expands variables, `for` loops and globs, resolves
every source to a repo file, and — the part that matters — **errors on
anything into a systemd directory it cannot resolve**. Silence is the
defect; an unresolved line is now a failure that names itself. On top
of that, every systemd unit file in the tree must be accounted for by
some installer line, so a unit added by an idiom the walker does not
model still cannot ship unseen: it fails as an orphan.

Drop-ins are merged into the unit before it is judged, and directives
are read with systemd's own semantics — last assignment wins, an empty
assignment RESETS the list — because `IPAddressDeny=any` followed later
by `IPAddressDeny=` is a unit with no IP filter at all.

Static assertion only. The runtime proof — actually attempting egress
from inside each shipped sandbox and watching it fail — is
`tests/e2e/egress-test.sh`, which takes its unit list from this file
(`--list no-egress`) and its drop-in list from `--dropins`, so the two
can never be two lists. This check is still worth having on its own:
the runtime test proves *the sandbox as a whole* blocks egress, which
stays true if `IPAddressDeny` is deleted from a unit that also has
`RestrictAddressFamilies=AF_UNIX`. Defence in depth only counts if
losing a layer is noticed, and only a static check can notice that.

Usage:
    check-egress-units.py             # the gate
    check-egress-units.py --explain   # print the classification table
    check-egress-units.py --installs  # the install-line resolution table
    check-egress-units.py --list no-egress|egress|exempt|cli
    check-egress-units.py --dropins <repo-relative unit path>
"""

import re
import shlex
import sys
from pathlib import Path

# ---------------------------------------------------------------- postures

# Binaries that must never reach the network. Value = why, in the terms
# the unit itself uses; if a daemon's answer to "what does it talk to"
# ever includes an address off this machine, it belongs in EGRESS and
# needs an ADR, not an edit here.
NO_EGRESS = {
    "lisa-inferenced": (
        "PLAN §5.1 model runtime. Serves loopback (7777 system, 7778 user) "
        "and a unix socket; the BYO remote tier is reached by handing the "
        "request to lisa-remoted over %t/lisa/remoted.sock, so the engine "
        "itself never opens a route off the machine."
    ),
    "lisa-contextd": (
        "PLAN §5.3 context fabric. D-Bus surface, local SQLite store. It "
        "reads the user's consented index — the daemon with the most to "
        "leak and the least reason to."
    ),
    "lisa-agentd": (
        "PLAN §5.4 Agent Bus. D-Bus plus per-app MCP unix sockets. Tool "
        "dispatch that could reach the network would make every tier and "
        "provenance decision advisory."
    ),
    "lisa-harnessd": (
        "ADR-0025 agent loop. Hosts the MODEL, and reaches its inferenced "
        "companion over %t/lisa/inferenced.sock — a unix socket, so the "
        "unit takes RestrictAddressFamilies=AF_UNIX and there is no IP "
        "socket to point anywhere (#288). It used to say 'allowed "
        "loopback (IPAddressAllow=localhost) to reach :7778', which "
        "described a confinement a user unit cannot have: a model "
        "endpoint that could be pointed at the internet by configuration "
        "is an egress channel with a config file for a guard."
    ),
    "lisa-notes": (
        "ADR-0013 first-party MCP server. Unix socket plus SQLite under "
        "$XDG_DATA_HOME. Every app tool provider inherits this posture."
    ),
    "lisa-overlayd": (
        "PLAN §5.7.1/§5.7.5 overlay backend (dev.lisaos.Overlay1, "
        "dev.lisaos.Voice1). Session bus, plus an OpenAI-compat POST to "
        "http://127.0.0.1:7778 — its inferenced companion, on loopback. "
        "The conversation leaves this machine, if at all, from inferenced "
        "through lisa-remoted. Until #294 it had NO unit at all: both "
        "activation files carried a bare Exec= and dbus-daemon forked it "
        "directly, so it was in no bucket here, not even out-of-scope."
    ),
    "xdg-desktop-portal-lisa": (
        "PLAN §5.5 trust boundary. It brokers fds and consent; it never "
        "carries model traffic, as its own unit says."
    ),
}

# Binaries permitted egress. Rule 5's positive half: this set should
# have exactly one entry that ships a unit, and adding to it is an ADR.
EGRESS = {
    "lisa-remoted": (
        "ADR-0010 / PLAN §5.11 — THE egress broker, and the only one. "
        "Every request it makes is ledgered with the `remote.` "
        "'leaves your hardware' marking behind default-deny per-scope "
        "consent."
    ),
    "lisa-modeld": (
        "PLAN §5.2 content-addressed model store. Fetches model "
        "artifacts. Ships no unit today (#275 item 3 stays open), so "
        "nothing below will match it — the entry is here so that the "
        "day it gets one, this file does not fail for the wrong reason."
    ),
}

# The `lisa` CLI is not a daemon and its units are one-shots. Recorded
# rather than policed, and the awkward part is recorded too.
NOT_A_DAEMON = {
    "lisa": (
        "The user-facing CLI (`lisa apps sync`, `lisa mail sync`, "
        "`lisa context sync-knowledge`), invoked as Type=oneshot. Rule 5 "
        "governs daemons; a CLI's egress is governed by the guard "
        "catalogue and by remoted. lisa-mail-sync.service reaches an "
        "IMAP host directly, and that is now a DECIDED exemption rather "
        "than a loose end recorded here: see ADR-0059. Rule 5 is about "
        "model traffic and about daemons that hold the user's context "
        "leaving with it; a mail client contacting the user's own server "
        "on the user's own credential is neither. The limit is in the "
        "ADR too — sending content the model composed is model traffic "
        "and is brokered. This string is not the place that argument "
        "lives (#286)."
    ),
}

# ------------------------------------------------------------------- debt
#
# THE TWO DEBT LISTS BELOW ARE RATCHETED (#293). Read this before adding
# to either.
#
# Both lists remove a unit from a check — EXEMPT from the static
# directive assertion AND, via `--list no-egress`, from the runtime
# suite in tests/e2e/egress-test.sh; USER_SCOPE_INET_DEBT from the
# user-scope confinement assertion. Until 2026-08-06 they were a
# convention with nothing enforcing it: re-adding one line blinded both
# layers at once, for free, green, in a file a reviewer reads as "the
# gate" rather than "the units". The self-retiring property 3a0fe1f
# claimed is real, but only in the direction of hardening — nothing at
# all resisted going the other way.
#
# The ceiling below is the resistance. `len()` of each list is asserted
# against it, so ADDING AN ENTRY FAILS unless the same commit also
# raises the ceiling — and that second line reads, in the diff, as
# "this change increases the number of Lisa daemons nothing confines
# from N to N+1", which is not a sentence anyone lands by accident. A
# ratchet is what this repo already uses for the same shape elsewhere;
# it is the strongest mechanism available to a gate that cannot reach
# the network to ask whether an issue is still open.
#
# Be exact about what this is NOT: it does not make an entry
# impossible, and it cannot. A two-line commit that raises the ceiling
# and adds the entry still lands. What it removes is the *free, green,
# one-line* version, and the reviewer-attention problem — the ceiling
# is the diff line that says what happened in plain numbers.
#
# Every ceiling is a number that may only go DOWN without argument.
DEBT_CEILING = {
    # Empty, and that is the point — see EXEMPT's own note. A ceiling of
    # zero means the ONLY way back to a non-empty EXEMPT is a commit
    # that says so in this dict first.
    "EXEMPT": 0,
    # Two: lisa-inferenced-dbus.service, which LISTENS on 127.0.0.1:7778,
    # and lisa-overlayd.service, which is one of the two libsoup callers
    # on the other end of that port. The count went 1 -> 2 on 2026-08-06
    # not because a daemon lost its confinement but because one that
    # never had any ENTERED this gate's population at all: until #294 the
    # overlay backend was spawned by dbus-daemon with no unit, so it was
    # unconfined AND uncounted. A number that rises when a hidden process
    # is made visible is the number doing its job; both entries retire
    # together when :7778 is socket-activated or the GJS callers move
    # onto dev.lisaos.Inference1 (#288).
    "USER_SCOPE_INET_DEBT": 2,
    # Three D-Bus activation records still spawned by dbus-daemon with
    # no unit. #294 fixed the two it named (Overlay1, Voice1); the check
    # that fixed them found three more of the same shape, all GUI
    # surfaces rather than backends. Named and counted rather than left
    # undiscovered — and a fourth cannot be added for free.
    "DBUS_UNSANDBOXED_DEBT": 3,
}

# No-egress binaries whose SHIPPED unit does not carry IPAddressDeny
# today. Each entry is a debt, not a dispensation — and the check FAILS
# if an exempt unit gains the directive, so the exemption deletes itself
# the moment the debt is paid. (Requiring the directive here instead
# would mean shipping a red gate, which teaches everyone to ignore it.)
# Empty, and that is the point. xdg-desktop-portal-lisa.service was the
# one entry: it carried the per-user hardening subset but no
# IPAddressDeny/RestrictAddressFamilies, on the component that IS the
# trust boundary (#285). The exemption lived for exactly as long as it
# took to harden the unit — the gate refused to pass with both the
# directive and the exemption present, which is the self-retiring
# mechanism working rather than a comment claiming it would.
EXEMPT = {}

# User-scope units that still permit AF_INET, where IPAddressDeny is a
# no-op (see the check below for the hardware measurement). An entry here
# is NOT confined — that is #288, a real hole, not a formality. They are
# listed rather than left failing because a permanently red gate teaches
# everyone to ignore it; every OTHER unit, and every future one, fails
# immediately. Like EXEMPT above, an entry whose unit has been fixed is
# itself an error, so the list cannot outlive the debt.
#
# lisa-harnessd.service WAS the first entry and is gone, which is the
# mechanism working: the model host now reaches inferenced over
# %t/lisa/inferenced.sock (the socket the daemon already served for
# lisa-contextd, #163) and carries RestrictAddressFamilies=AF_UNIX, so
# the gate refused to pass while the entry was still here.
#
# The one that remains is a LISTENER, not a caller, and the difference is
# why it could not follow: lisa-inferenced binds 127.0.0.1:7778, and two
# GJS surfaces are still its clients over that port —
# shell/assistant/lisa-assistant.js (GET /v1/models) and
# shell/overlay-extension/backend/lisa-overlayd.js (POST
# /v1/chat/completions, streaming). Both go through libsoup, which has no
# unix-socket transport. Two ways out, neither of them a unit edit alone:
# socket-activate the port so the user manager owns the listening fd
# (RestrictAddressFamilies filters socket(2), never accept(2) on an
# inherited fd), or move the two callers onto the unix socket or
# dev.lisaos.Inference1.
USER_SCOPE_INET_DEBT = {
    "lisa-inferenced-dbus.service": (
        "LISTENS on 127.0.0.1:7778 for two libsoup callers in shell/ that "
        "have no unix-socket transport. Its 47fa7c6 'egress sandbox' "
        "claimed a confinement user scope cannot give. #288."
    ),
    "lisa-overlayd.service": (
        "One of those two callers, seen from the other end: it POSTs "
        "/v1/chat/completions to 127.0.0.1:7778 through libsoup, which "
        "has no unix-socket transport, so AF_INET cannot be dropped "
        "without taking the overlay's chat lane with it. New to this "
        "list because it is new to this gate: before #294 it ran with no "
        "unit and no sandbox of any kind, which is strictly worse than "
        "an entry here. Retires with lisa-inferenced-dbus.service."
    ),
}

# D-Bus activation records that STILL have no `SystemdService=`, so
# dbus-daemon forks them itself and the process runs with no unit and no
# sandbox at all (see dbus_activation() below).
#
# #294 named dev.lisaos.Overlay1 and dev.lisaos.Voice1, and those are
# fixed: both now route through lisa-overlayd.service. The check written
# to keep them fixed immediately found three more records of the same
# shape — which is the point of writing a check instead of editing two
# files. They are listed rather than left failing, for the reason stated
# at DEBT_CEILING: a permanently red gate is a gate everyone learns to
# skip. Each is a real hole, and all three are GUI surfaces rather than
# headless backends, which changes the cost and not the classification:
# two of them are MCP tool providers, so "no sandbox" applies to an
# agent surface.
#
# Keyed by the record's repo path, so moving the file is a decision
# somebody makes on purpose rather than an exemption that follows it.
DBUS_UNSANDBOXED_DEBT = {
    "shell/consent/dev.lisaos.Consent1.service": (
        "The consent surface. Its own comment is a security decision "
        "about the Exec line (#289) and says nothing about the unit; "
        "activation still goes through dbus-daemon."
    ),
    "shell/assistant/app.lisaos.Assistant.service": (
        "The Assistant window (#210 added the activation record so the "
        "Spotlight hand-off works when the app is closed). A GTK app, "
        "and an agent surface."
    ),
    "apps/preview/org.gnome.NautilusPreviewer.service": (
        "Preview answering Nautilus's Space key. Nautilus dials the "
        "versionless name and ping-gates it, so the record has to exist; "
        "the unit does not yet."
    ),
}

# Rule 5 names three daemons. If discovery stops finding a unit for one
# of them — deleted, renamed, moved out of the installers — the check
# above would go quietly green over an empty set. This is the floor.
REQUIRED_NO_EGRESS_COVERAGE = ("lisa-inferenced", "lisa-contextd", "lisa-agentd")

# ...and the positive half needs a shipped unit too, or "only remoted"
# is an absence rather than a statement.
REQUIRED_EGRESS_COVERAGE = ("lisa-remoted",)

DIRECTIVE = "IPAddressDeny=any"


# ---------------------------------------------------------------- discovery
#
# The installers are INTERPRETED, not pattern-matched (#291). What the
# first version did — take every whitespace token on a line mentioning
# `systemd/system/`, keep the ones that happen to name a repo file —
# looks like discovery and is really a filter, and everything it fails
# to recognise falls out of the population without a word. Five ordinary
# idioms fell out that way, one of them already in the shipping
# PKGBUILD, and sixteen real install lines resolved to nothing in
# silence.
#
# So: a small shell walker with an environment, `for` loops, globs and
# quoting, and one rule that matters more than any of the parsing —
# **an install into a systemd directory that cannot be resolved is an
# ERROR**. The gate may not know; it may not be quiet about not knowing.

# Where a payload has to land before this file cares about it. All four
# prefixes, because systemd reads all four and the image build already
# uses two of them (`mkosi.extra/etc/systemd/system/` exists today; the
# old `usr/lib/systemd/*/*.service` glob could not see it).
# The trailing path is OPTIONAL: `install -Dm644 x -t "$pkgdir/usr/lib/
# systemd/user"` names the directory with no slash after it, and a
# pattern that demanded one skipped the line entirely.
SYSTEMD_DEST = re.compile(
    r"(?:^|/)(?:etc|run|lib|usr/lib|usr/local/lib)/systemd/(system|user)(?:/(.*))?$"
)

# The one substring a statement must contain before the walker judges
# it. Deliberately the destination directory rather than the file
# extension: a unit's SOURCE may be named anything, and routinely is
# (lisa-remoted-user.service installs as lisa-remoted.service). No
# trailing slash, for the same reason SYSTEMD_DEST does not require one.
SYSTEMD_MARKERS = ("systemd/system", "systemd/user")

# THE SECOND SPAWN PATH (#294). `dbus-daemon` starts a service itself
# when its activation record carries only `Exec=`; the process gets no
# unit, therefore no sandbox, and — because this file discovers its
# population from systemd install lines — no row here at all. Not
# `no-egress`, not `egress`, not `out-of-scope`: absent. That is exactly
# the failure this file's docstring names when it says a unit's
# `[Install]` lies by omission, arriving through the other door.
#
# So activation records are discovered from the same install lines, by
# the same interpreter, and one is asserted about them: an `Exec=` that
# names a Lisa binary must also name a `SystemdService=`.
DBUS_MARKER = "dbus-1/services"

# makepkg's own variables. `%PKGDIR%` rather than a real path so that a
# destination which still holds an unexpanded `$something` is obvious.
MAKEPKG_ENV = {"pkgdir": "%PKGDIR%", "srcdir": "%SRCDIR%", "startdir": "%STARTDIR%"}

VAR_RE = re.compile(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}|\$([A-Za-z_][A-Za-z0-9_]*)")
ASSIGN_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)=(.*)$", re.S)

COPIERS = ("install", "cp", "rsync")
LINKERS = ("ln",)
MAKEDIRS = ("mkdir",)
# Shell keywords the walker steps over rather than mistaking for a
# command. `done`/`fi` land here; `for`/`while` are handled above.
KEYWORDS = frozenset(
    "do done then fi else elif esac fi case function local return "
    "{ } ( ) ;; ! time".split()
)

# Options that swallow the NEXT argument, so it is not a source.
OPTS_WITH_ARG = frozenset(
    ["-t", "--target-directory", "-m", "--mode", "-o", "--owner", "-g", "--group",
     "-S", "--suffix", "-Z", "--context", "--backup"]
)

# Unit suffixes systemd knows. A destination under a systemd directory
# ending in something else is a shape this walker does not model, and
# says so instead of ignoring it.
UNIT_SUFFIXES = (
    ".service", ".socket", ".timer", ".target", ".path", ".mount",
    ".automount", ".swap", ".slice", ".scope", ".device",
)


def _logical_lines(text):
    """(first line number, joined text) — backslash continuations joined."""
    out, buf, start = [], "", 1
    for n, raw in enumerate(text.splitlines(), 1):
        if not buf:
            start = n
        if raw.endswith("\\"):
            buf += raw[:-1] + " "
            continue
        out.append((start, buf + raw))
        buf = ""
    if buf:
        out.append((start, buf))
    return out


def _split_statements(line):
    """Break a logical line on `;`, `&`, `|`, `&&`, `||` outside quotes."""
    parts, cur, quote, i = [], "", "", 0
    while i < len(line):
        c = line[i]
        if quote:
            cur += c
            if c == quote:
                quote = ""
            i += 1
            continue
        if c in "\"'":
            quote = c
            cur += c
            i += 1
            continue
        if c == "\\" and i + 1 < len(line):
            cur += line[i:i + 2]
            i += 2
            continue
        if c == "#" and (not cur or cur[-1].isspace()):
            break  # comment runs to end of line
        if line[i:i + 2] in ("&&", "||"):
            parts.append(cur)
            cur = ""
            i += 2
            continue
        if c in ";|&":
            parts.append(cur)
            cur = ""
            i += 1
            continue
        cur += c
        i += 1
    parts.append(cur)
    return parts


def _tokenize(pkgbuild, text, errors):
    """[(line number, argv)] for every simple command in a PKGBUILD.

    A statement shlex cannot tokenize is skipped UNLESS it names a
    systemd directory, in which case it is an error: not understanding a
    line that installs a unit is the failure mode this whole file is
    about.
    """
    stmts = []
    for lineno, line in _logical_lines(text):
        for part in _split_statements(line):
            if not part.strip():
                continue
            try:
                argv = shlex.split(part, comments=True, posix=True)
            except ValueError as exc:
                if any(m in part for m in SYSTEMD_MARKERS):
                    errors.append(
                        f"{pkgbuild}:{lineno}: cannot tokenize a statement that "
                        f"names a systemd directory ({exc}): {part.strip()[:120]}"
                    )
                continue
            stmts.append((lineno, argv))
    return stmts


def _expand(word, env):
    """Substitute $NAME / ${NAME} from env. Unknown names are left as-is
    so that the `$` survives into the resolver and fails loudly."""
    def sub(match):
        name = match.group(1) or match.group(2)
        return env.get(name, match.group(0))
    return VAR_RE.sub(sub, word)


def _take_block(stmts, i):
    """Statements up to the matching `done`; returns (body, index after)."""
    body, depth = [], 0
    while i < len(stmts):
        argv = stmts[i][1]
        head = argv[0] if argv else ""
        if head in ("for", "while", "until", "select"):
            depth += 1
        elif head == "done":
            if depth == 0:
                return body, i + 1
            depth -= 1
        body.append(stmts[i])
        i += 1
    return body, i


def _walk(stmts, env, visit):
    """Run the statement list, calling visit(lineno, argv) per command.

    `for` loops are expanded — PKGBUILD:262 already ships two drop-ins
    through one, and the old discovery could not see either. `if`/`case`
    bodies are walked unconditionally, which over-collects rather than
    under-collects: this gate would rather judge a unit that a build
    might skip than miss one it ships.
    """
    i = 0
    while i < len(stmts):
        lineno, argv = stmts[i]
        if not argv:
            i += 1
            continue
        head = argv[0]
        if head in ("for", "select") and "in" in argv:
            var = argv[1]
            values = [_expand(w, env) for w in argv[argv.index("in") + 1:]]
            body, i = _take_block(stmts, i + 1)
            for value in values:
                child = dict(env)
                child[var] = value
                _walk(body, child, visit)
            continue
        if head in ("while", "until"):
            body, i = _take_block(stmts, i + 1)
            _walk(body, dict(env), visit)
            continue
        if head in KEYWORDS or head in ("if", "for", "while", "until", "select"):
            i += 1
            continue
        # Leading `NAME=value` words: an assignment statement mutates the
        # environment; an assignment PREFIX applies to that command only.
        assigns, j = {}, 0
        while j < len(argv):
            match = ASSIGN_RE.match(argv[j])
            if not match:
                break
            value = _expand(match.group(2), env)
            if not value.startswith("("):  # arrays are not paths
                assigns[match.group(1)] = value
            j += 1
        if j == len(argv):
            env.update(assigns)
            i += 1
            continue
        local = dict(env, **assigns) if assigns else env
        visit(lineno, [_expand(w, local) for w in argv[j:]])
        i += 1


def _split_opts(argv):
    """(options, operands) for an install/cp-shaped command line."""
    opts, args, take_next = [], [], False
    for tok in argv[1:]:
        if take_next:
            opts.append(tok)
            take_next = False
            continue
        if tok == "--":
            continue
        if tok.startswith("-") and tok != "-":
            opts.append(tok)
            if tok in OPTS_WITH_ARG:
                take_next = True
            continue
        args.append(tok)
    return opts, args


def _resolve_source(root, pkgbuild, word):
    """Repo files a source word names, or [] if it names none.

    Two bases, because PKGBUILDs use both: the extracted source tree is
    a copy of the repo (`cd "$pkgbase-$pkgver"` then
    `os/packages/lisa/x.service`), and a file may also be named relative
    to the PKGBUILD's own directory.
    """
    hits = []
    for base in (root, pkgbuild.parent):
        if any(c in word for c in "*?["):
            hits.extend(p for p in sorted(base.glob(word)) if p.is_file())
        else:
            candidate = base / word
            if candidate.is_file():
                hits.append(candidate)
            elif candidate.is_dir():
                hits.extend(p for p in sorted(candidate.rglob("*")) if p.is_file())
        if hits:
            break
    return hits


class Landing:
    """One repo file landing at one path inside a systemd directory."""

    __slots__ = ("src", "scope", "rel", "kind", "unit", "origin")

    def __init__(self, src, scope, rel, kind, unit, origin):
        self.src = src        # Path in the repo, or None for mkosi trees
        self.scope = scope    # "system" | "user"
        self.rel = rel        # path under systemd/<scope>/
        self.kind = kind      # "unit" | "dropin" | "other-unit" | "wants"
        self.unit = unit      # installed unit filename this belongs to
        self.origin = origin  # "os/packages/lisa/PKGBUILD:143" or a tree path


def _classify_dest(scope, rel, src, origin, errors):
    """A Landing for a path under systemd/<scope>/, or None with an error."""
    rel = rel.strip("/")
    if not rel:
        return None
    match = re.fullmatch(r"([^/]+\.service)\.d/[^/]+\.conf", rel)
    if match:
        return Landing(src, scope, rel, "dropin", match.group(1), origin)
    if re.fullmatch(r"[^/]+\.service", rel):
        return Landing(src, scope, rel, "unit", rel, origin)
    if re.fullmatch(r"[^/]+\.(?:target|service|socket|timer|path)\.(?:wants|requires)/[^/]+", rel):
        return Landing(src, scope, rel, "wants", Path(rel).name, origin)
    if any(rel.endswith(sfx) for sfx in UNIT_SUFFIXES) and "/" not in rel:
        return Landing(src, scope, rel, "other-unit", rel, origin)
    errors.append(
        f"{origin}: installs into systemd/{scope}/ at a path this check does "
        f"not model — `{rel}`. Teach _classify_dest() what it is; a shape "
        f"nobody modelled is a shape nobody checked."
    )
    return None


def _pkgbuild_landings(root, pkgbuild, landings, table, errors, activations):
    text = pkgbuild.read_text()
    rel_pkgbuild = pkgbuild.relative_to(root)
    stmts = _tokenize(rel_pkgbuild, text, errors)

    def visit(lineno, argv):
        into_systemd = any(m in tok for tok in argv for m in SYSTEMD_MARKERS)
        into_dbus = any(DBUS_MARKER in tok for tok in argv)
        if not into_systemd and not into_dbus:
            return
        origin = f"{rel_pkgbuild}:{lineno}"
        head = Path(argv[0]).name
        opts, args = _split_opts(argv)

        if into_dbus and not into_systemd:
            # A D-Bus activation record (#294). Same resolution, same
            # "an unresolved install is an error" rule; the assertion
            # itself is in dbus_activation() below.
            if head in COPIERS:
                if "-t" in opts or "--target-directory" in opts:
                    sources = args
                elif len(args) >= 2:
                    sources = args[:-1]
                else:
                    sources = []
                for word in sources:
                    if "$" in word or "%SRCDIR%" in word or "%STARTDIR%" in word:
                        errors.append(
                            f"{origin}: the source `{word}` installed into "
                            f"{DBUS_MARKER}/ still holds an unexpanded shell "
                            f"expansion, so this check cannot tell WHICH "
                            f"activation record ships."
                        )
                        continue
                    hits = _resolve_source(root, pkgbuild, word)
                    if not hits:
                        errors.append(
                            f"{origin}: the source `{word}` is installed into "
                            f"{DBUS_MARKER}/ and resolves to no file in the "
                            f"repo. An activation record this check cannot "
                            f"read is a second spawn path it cannot see (#294)."
                        )
                        continue
                    for src in hits:
                        activations.append((src.resolve(), origin))
            return

        if head in MAKEDIRS or (head == "install" and ("-d" in opts or "-D" in opts and not args[:-1])):
            if head in MAKEDIRS or "-d" in opts:
                table.append((origin, "mkdir", " ".join(args), "-"))
                return

        if head in LINKERS:
            # `ln -s ../x.service .../default.target.wants/x.service` —
            # the enable symlink. Its target must be a unit something
            # else installs; cross-checked once discovery is complete.
            target = args[0] if args else ""
            dest = args[-1] if len(args) > 1 else ""
            table.append((origin, "symlink", target, dest))
            match = SYSTEMD_DEST.search(dest.replace("%PKGDIR%", ""))
            if match:
                landing = _classify_dest(match.group(1), match.group(2) or "", None,
                                         origin, errors)
                if landing is not None:
                    landing.kind = "wants"
                    landing.unit = Path(target).name
                    landings.append(landing)
            return

        if head not in COPIERS:
            errors.append(
                f"{origin}: `{head}` puts something into a systemd directory "
                f"and this check does not model that command, so whatever it "
                f"installs is invisible here. Add it to COPIERS/LINKERS/"
                f"MAKEDIRS or stop using it: "
                f"{' '.join(argv)[:140]}"
            )
            return

        if "-t" in opts or "--target-directory" in opts:
            key = "-t" if "-t" in opts else "--target-directory"
            dest = opts[opts.index(key) + 1]
            sources, into_dir = args, True
        elif len(args) < 2:
            errors.append(
                f"{origin}: `{head}` with fewer than two operands "
                f"({args}) — cannot tell source from destination."
            )
            return
        else:
            dest, sources = args[-1], args[:-1]
            into_dir = len(sources) > 1 or dest.endswith("/")

        dest_match = SYSTEMD_DEST.search(dest.replace("%PKGDIR%", ""))
        if dest_match is None:
            # A source that mentions a systemd path but a destination
            # that does not: not our business, but say so in the table
            # rather than dropping it.
            table.append((origin, "not-a-systemd-dest", " ".join(sources), dest))
            return
        scope, rel_dest = dest_match.group(1), dest_match.group(2) or ""
        if not rel_dest:
            into_dir = True   # the destination named a directory, not a file

        for word in sources:
            if "$" in word or "%SRCDIR%" in word or "%STARTDIR%" in word:
                errors.append(
                    f"{origin}: the source `{word}` still holds an unexpanded "
                    f"shell expansion, so this check cannot tell WHICH file "
                    f"lands in systemd/{scope}/. An install line it cannot "
                    f"resolve is a unit it cannot see (#291)."
                )
                table.append((origin, "UNRESOLVED", word, rel_dest))
                continue
            hits = _resolve_source(root, pkgbuild, word)
            if not hits:
                errors.append(
                    f"{origin}: the source `{word}` resolves to no file in the "
                    f"repo, yet it is installed into systemd/{scope}/. Either "
                    f"the path is wrong or this check cannot see it; both are "
                    f"failures, and the old behaviour — dropping the line in "
                    f"silence — is what #291 is."
                )
                table.append((origin, "UNRESOLVED", word, rel_dest))
                continue
            for src in hits:
                target_rel = (
                    f"{rel_dest.rstrip('/')}/{src.name}" if into_dir else rel_dest
                )
                landing = _classify_dest(scope, target_rel, src.resolve(),
                                         origin, errors)
                if landing is None:
                    continue
                landings.append(landing)
                table.append((origin, landing.kind,
                              str(src.resolve().relative_to(root)),
                              f"systemd/{scope}/{landing.rel}"))

    _walk(stmts, dict(MAKEPKG_ENV), visit)


def _tree_landings(root, landings, errors):
    """Units and drop-ins baked straight into an image tree.

    Every systemd directory, not just `usr/lib/systemd/*/` — the old
    glob could not see `mkosi.extra/etc/systemd/system/`, which the
    image build already populates.
    """
    for tree in ("mkosi.extra", "initrd-overlay"):
        for base in sorted((root / "os").rglob(tree)):
            if not base.is_dir():
                continue
            for path in sorted(base.rglob("*")):
                if path.is_symlink() or not path.is_file():
                    continue
                rel = path.relative_to(base).as_posix()
                match = SYSTEMD_DEST.search("/" + rel)
                if match is None:
                    continue
                origin = str(path.relative_to(root))
                landing = _classify_dest(match.group(1), match.group(2) or "",
                                         path.resolve(), origin, errors)
                if landing is not None:
                    landings.append(landing)


def discover(root: Path):
    """(units, dropins, table, errors, activations).

    units       — [(unit file in the repo, installed name, scope, origin)]
    dropins     — {(scope, installed unit name): [drop-in files, applied order]}
    table       — every install line into a systemd directory, resolved
    errors      — the lines that could NOT be resolved, and why
    activations — [(D-Bus activation record in the repo, origin)] (#294)
    """
    landings, table, errors, activations = [], [], [], []
    for pkgbuild in sorted((root / "os" / "packages").rglob("PKGBUILD")):
        _pkgbuild_landings(root, pkgbuild, landings, table, errors, activations)
    _tree_landings(root, landings, errors)

    units, dropins, installed_names = [], {}, {}
    for landing in landings:
        if landing.kind == "unit" and landing.src is not None:
            units.append((landing.src, landing.unit, landing.scope, landing.origin))
            installed_names.setdefault(landing.scope, set()).add(landing.unit)
        elif landing.kind == "dropin" and landing.src is not None:
            dropins.setdefault((landing.scope, landing.unit), []).append(
                (landing.rel, landing.src))
        elif landing.kind == "other-unit":
            installed_names.setdefault(landing.scope, set()).add(landing.unit)

    # An enable symlink pointing at a unit nothing installs is a unit
    # that either does not exist or is installed by an idiom this walker
    # cannot see. Either way it is not the silence it used to be.
    for landing in landings:
        if landing.kind != "wants" or not landing.unit.endswith(".service"):
            continue
        if landing.unit not in installed_names.get(landing.scope, ()):
            errors.append(
                f"{landing.origin}: enables `{landing.unit}` in "
                f"systemd/{landing.scope}/, but nothing discovered here "
                f"installs a unit by that name. Either it ships by an idiom "
                f"this check cannot read — in which case it also ships "
                f"unchecked — or the symlink dangles."
            )

    # Drop-ins are applied in filename order across all drop-in dirs,
    # which is what systemd does and what makes the merge below match
    # `systemctl cat`.
    for key in dropins:
        dropins[key].sort(key=lambda pair: Path(pair[0]).name)

    return units, dropins, table, errors, activations


def dbus_activation(root, activations, installed_units, errors):
    """Every shipped activation record for a Lisa binary names a unit (#294).

    `dbus-daemon` will start a service itself when the record carries
    only `Exec=`. That process has no unit, so it has no sandbox — no
    RestrictAddressFamilies, no NoNewPrivileges, nothing — and no row in
    this gate, because the gate's population comes from systemd install
    lines and there is no unit to install. dev.lisaos.Overlay1 and
    dev.lisaos.Voice1 shipped that way while three activation records in
    the same directory carried `SystemdService=`.

    The rule is narrow on purpose. A record whose `Exec=` is somebody
    else's binary is not rule 5's business and is left alone; a record
    for a Lisa binary must route activation through systemd, and the
    unit it names must be one discovery actually installs — otherwise
    the `SystemdService=` line is a promise to a unit that does not
    exist, which fails closed at runtime and reads as fixed here.
    """
    for path, origin in sorted(set(activations)):
        text = path.read_text(errors="replace")
        rel = path.relative_to(root)
        exec_line = systemd_service = None
        for raw in text.splitlines():
            key, _, value = raw.partition("=")
            key, value = key.strip(), value.strip()
            if key == "Exec" and exec_line is None:
                exec_line = value
            elif key == "SystemdService" and systemd_service is None:
                systemd_service = value
        if not exec_line:
            continue
        argv = exec_line.split()
        head = _program_name(argv[0].lstrip("-+!:@")) if argv else None
        if not head or not is_lisa_binary(head):
            continue
        if not systemd_service:
            if str(rel) in DBUS_UNSANDBOXED_DEBT:
                continue
            errors.append(
                f"{rel} (installed by {origin}): a D-Bus activation record "
                f"running `{head}` with NO SystemdService=. dbus-daemon will "
                f"fork it directly, so it gets no unit, no sandbox and no row "
                f"in this gate — not even out-of-scope. Give it a unit and "
                f"name it here (#294)."
            )
        elif str(rel) in DBUS_UNSANDBOXED_DEBT:
            errors.append(
                f"{rel}: now names SystemdService={systemd_service} and is "
                f"still listed in DBUS_UNSANDBOXED_DEBT. Delete the entry — "
                f"the list exists to be deleted, and one that outlives its "
                f"debt is how the next reader learns to distrust it."
            )
        elif systemd_service not in installed_units:
            errors.append(
                f"{rel} (installed by {origin}): names "
                f"SystemdService={systemd_service}, which nothing discovered "
                f"here installs. Activation would fail at runtime and this "
                f"check would read as satisfied."
            )


# Repo files that ARE systemd fragments and are deliberately installed
# nowhere. Empty, and it must stay a named list rather than a silence:
# an orphan is either dead weight or a unit shipping by a route this
# check cannot read.
NOT_SHIPPED = {}


def orphan_fragments(root: Path, units, dropins, errors):
    """Every systemd fragment in the tree is installed by something.

    The counterpart to discovery, and the reason a unit added by an
    idiom the walker does not model still cannot ship unseen: it lands
    here as an orphan. `.service` files that are D-Bus activation
    records (`[D-BUS Service]`) are not systemd units and are skipped by
    reading them, not by name.
    """
    shipped = {src.resolve() for src, _n, _s, _o in units}
    for files in dropins.values():
        shipped.update(src.resolve() for _rel, src in files)

    for path in sorted((root / "os" / "packages").rglob("*")):
        if not path.is_file() or path.suffix not in (".service", ".conf"):
            continue
        try:
            text = path.read_text()
        except (UnicodeDecodeError, OSError):
            continue
        sections = set(re.findall(r"(?m)^\s*\[([^\]]+)\]\s*$", text))
        if not sections & {"Unit", "Service", "Install", "Socket", "Timer"}:
            continue  # D-Bus activation file, gschema override, ini of some other kind
        rel = str(path.relative_to(root))
        if path.resolve() in shipped or rel in NOT_SHIPPED:
            continue
        errors.append(
            f"{rel}: is a systemd fragment (sections {sorted(sections)}) that "
            f"no installer line in os/packages/**/PKGBUILD was seen to "
            f"install. Either it ships through an idiom this check cannot "
            f"read — and therefore ships UNCHECKED, which is #291 — or it is "
            f"dead weight. Install it, delete it, or name it in NOT_SHIPPED "
            f"with the reason."
        )


def service_lines(text: str):
    """([Service] directive, value, line number) for uncommented lines.

    Backslash continuations are joined, because systemd joins them: a
    multi-line `ExecStart=/bin/sh -c '\\ ... '` is ONE assignment, and
    reading its continuation lines as separate directives invents
    directives the unit does not have.
    """
    section = None
    for line_no, raw in _logical_lines(text):
        line = raw.strip()
        if line.startswith("#") or line.startswith(";") or not line:
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1]
            continue
        if section != "Service" or "=" not in line:
            continue
        key, value = line.split("=", 1)
        yield key.strip(), value.strip(), line_no


def effective_values(text: str, key: str):
    """(was it ever set, the values in force) — systemd's own semantics.

    The list directives this file reads (`IPAddressDeny`,
    `IPAddressAllow`, `RestrictAddressFamilies`) APPEND across repeated
    assignments, and an EMPTY assignment resets the list to nothing.
    The first version of this check asked `any(key == ... and value ==
    "any")` — first match wins — so

        IPAddressDeny=any
        IPAddressDeny=

    read as a unit with an IP filter, while systemd reads it as a unit
    with none (#292). On lisa-harnessd, where the family filter must
    allow AF_UNIX only and IPAddressDeny is the second layer, that is
    the difference between defence in depth and a comment.
    """
    values, seen = [], False
    for k, value, _ in service_lines(text):
        if k != key:
            continue
        if value == "":
            values, seen = [], False
        else:
            values.extend(value.split())
            seen = True
    return seen, values


def merged_text(unit: Path, dropin_files) -> str:
    """The unit as systemd assembles it: the file, then its drop-ins.

    A `.service.d/*.conf` that hands the network back was invisible to
    this gate and to the runtime harness at the same time (#292) — the
    static layer never opened a `.conf`, and egress-test.sh reconstructed
    the sandbox from the pristine unit file. Appending them here, in the
    order systemd applies them, is what makes both layers see the unit
    the machine actually runs.
    """
    parts = [unit.read_text()]
    for _rel, path in dropin_files:
        parts.append(f"\n# --- drop-in: {path}\n{path.read_text()}")
    return "".join(parts)


# argv[0] values that are a wrapper rather than the program. A Lisa
# daemon behind one erased the daemon from this gate's population
# entirely: `ExecStart=/bin/sh -c 'exec /usr/bin/lisa-contextd'` was
# filed under "runs something else, out of rule 5's scope" (#292).
WRAPPERS = frozenset(
    "sh bash dash ash zsh ksh env setsid nohup stdbuf timeout runuser "
    "setpriv chrt ionice nice systemd-inhibit".split()
)

# Directories a real program lives in. Without this, scanning a wrapper's
# argv for `lisa-*` matches `/run/lisa-esp` in lisa-boot-report.service
# and invents a daemon.
BIN_DIRS = ("/usr/bin", "/bin", "/usr/local/bin", "/usr/sbin", "/sbin",
            "/usr/lib/lisa", "/usr/libexec")

# OUR OWN launchers, whose first argument names the surface that will
# actually run. Distinct from WRAPPERS above, which are REFUSED: the
# objection to `sh -c` is that it hides the daemon from this gate and
# the real argv from the Ledger, and neither is true here. `lisa-app`
# takes a declared relative path under the apps tree and execs it, so
# the surface's name is right there on the ExecStart line — what the
# launcher varies is only WHICH copy of that surface runs (ADR-0020's
# whole point: `lisa apps update` must take effect without a reboot).
# The sandbox is the unit's and survives the exec regardless.
#
# So the posture is keyed on the surface (`lisa-overlayd`) rather than
# on the launcher. Keying it on `lisa-app` would be the mistake this
# file already names elsewhere — "a posture for whichever one this line
# happened to pick first" — because every GJS surface on the system is
# started through the same launcher (#294).
LAUNCHERS = frozenset(["lisa-app"])


def _launched_surface(argv):
    """The Lisa surface a LAUNCHERS argv[0] runs, e.g. `lisa-overlayd`."""
    for arg in argv[1:]:
        arg = arg.strip("'\"")
        if arg.startswith("-"):
            continue
        name = arg.rsplit("/", 1)[-1]
        if name.endswith(".js"):
            name = name[:-3]
        return name or None
    return None


def _program_name(token: str):
    """The program a token names, or None if it is not a program at all."""
    token = token.strip("'\"")
    if not token:
        return None
    if "/" not in token:
        return token
    if not token.startswith("/"):
        return None
    parent = token.rsplit("/", 1)[0]
    return token.rsplit("/", 1)[1] if parent in BIN_DIRS else None


def exec_programs(text: str):
    """(binaries, wrapped) over EVERY ExecStart= line in the unit.

    Both halves are #292. Reading only the FIRST ExecStart let a second
    one run the daemon unseen; taking argv[0] literally let any wrapper
    hide it. systemd allows `-`, `+`, `!`, `!!`, `:` and `@` prefixes on
    the command, stripped here or a `-/usr/bin/lisa` reads as a binary
    called `-lisa`.
    """
    names, wrapped = [], False
    for key, value, _ in service_lines(text):
        if key != "ExecStart" or not value:
            continue
        argv = value.split()
        head = _program_name(argv[0].lstrip("-+!:@"))
        if head in LAUNCHERS:
            # Classify the SURFACE, not the launcher every surface
            # shares (#294). An unresolvable one falls through to the
            # launcher's own name, which has no posture — so it fails
            # loudly rather than passing as something classified.
            surface = _launched_surface(argv)
            if surface and is_lisa_binary(surface):
                names.append(surface)
                continue
        if head and is_lisa_binary(head):
            names.append(head)
            continue
        if head in WRAPPERS:
            hits = [n for n in (_program_name(a) for a in argv[1:])
                    if n and is_lisa_binary(n)]
            if hits:
                wrapped = True
                names.extend(hits)
                continue
        if head and head not in names:
            names.append(head)
    return names, wrapped


def is_lisa_binary(name: str) -> bool:
    return name == "lisa" or name.startswith("lisa-") or name == "xdg-desktop-portal-lisa"


def inet_allowed(text: str) -> bool:
    """Can this unit open an IP socket at all?

    No `RestrictAddressFamilies=` means every family, so absence is the
    permissive answer — the opposite of how a missing directive usually
    reads, and the reason this returns True for `none`.
    """
    seen, families = effective_values(text, "RestrictAddressFamilies")
    if not seen:
        return True
    if any(f.startswith("~") for f in families):
        denied = {f.lstrip("~") for f in families}
        return not {"AF_INET", "AF_INET6"} <= denied
    return any(f in ("AF_INET", "AF_INET6") for f in families)


class Row:
    """One shipped unit, judged."""

    __slots__ = ("unit", "installed", "scope", "binary", "posture", "has",
                 "user_scope", "inet", "dropins", "wrapped", "origin")

    def __init__(self, **kw):
        for slot in self.__slots__:
            setattr(self, slot, kw.get(slot))


def classify(root: Path):
    """(rows, errors) for every unit the installers place."""
    units, dropins, _table, errors, activations = discover(root)
    orphan_fragments(root, units, dropins, errors)
    dbus_activation(root, activations,
                    {installed for _u, installed, _s, _o in units}, errors)

    rows, seen = [], set()
    for unit, installed, scope, origin in sorted(units, key=lambda u: str(u[0])):
        key = (unit, installed, scope)
        if key in seen:
            continue
        seen.add(key)
        applied = dropins.get((scope, installed), [])
        text = merged_text(unit, applied)
        binaries, wrapped = exec_programs(text)
        lisa = [b for b in binaries if is_lisa_binary(b)]
        row = Row(unit=unit, installed=installed, scope=scope,
                  dropins=[p for _rel, p in applied], wrapped=wrapped,
                  origin=origin,
                  has="any" in effective_values(text, "IPAddressDeny")[1],
                  user_scope=(scope == "user"), inet=inet_allowed(text))
        if not lisa:
            row.binary = binaries[0] if binaries else None
            row.posture = "out-of-scope"
            rows.append(row)
            continue
        row.binary = lisa[0]
        if len(set(lisa)) > 1:
            errors.append(
                f"{unit.relative_to(root)}: ExecStart runs more than one Lisa "
                f"binary ({sorted(set(lisa))}). One unit, one daemon, or the "
                f"posture below is a posture for whichever one this line "
                f"happened to pick first."
            )
        if row.binary in NO_EGRESS:
            row.posture = "no-egress"
        elif row.binary in EGRESS:
            row.posture = "egress"
        elif row.binary in NOT_A_DAEMON:
            row.posture = "cli"
        else:
            row.posture = "UNCLASSIFIED"
        rows.append(row)
    return rows, errors


def main(argv) -> int:
    root = Path(__file__).resolve().parents[2]

    if "--installs" in argv:
        # The resolution table (#291). Every install line into a systemd
        # directory, and what it resolved to. `UNRESOLVED` rows are the
        # ones the old discovery dropped in silence; there must be none,
        # and if there are, the gate is already failing on them.
        _u, _d, table, errs, _a = discover(root)
        print(f"{'ORIGIN':34} {'KIND':18} {'SOURCE':52} DESTINATION")
        for origin, kind, src, dest in table:
            print(f"{origin:34} {kind:18} {src:52} {dest}")
        print(f"\n{len(table)} install line(s) into systemd directories, "
              f"{sum(1 for r in table if r[1] == 'UNRESOLVED')} unresolved, "
              f"{len(errs)} discovery error(s).")
        return 0

    rows, errors = classify(root)

    if "--dropins" in argv:
        # The runtime harness (tests/e2e/egress-test.sh) asks this for
        # each unit it is about to attack, so the sandbox it applies is
        # the one systemd would assemble — unit file PLUS drop-ins
        # (#292). Without it the harness reconstructs the pristine unit
        # and reports "blocked" for a unit the machine runs unblocked.
        want = argv[argv.index("--dropins") + 1]
        for row in rows:
            if str(row.unit.relative_to(root)) == want:
                for path in row.dropins:
                    print(path.relative_to(root))
        return 0

    if "--list" in argv:
        want = argv[argv.index("--list") + 1]
        for row in rows:
            # `no-egress` is what the runtime harness attacks, so an
            # exempt unit must not appear in it: it demonstrably HAS
            # egress today (tests/e2e/egress-test.sh watched the portal
            # reach the internet), and a harness that failed on a debt
            # this file already records would just be reporting the
            # same thing twice, red.
            exempt = row.unit.name in EXEMPT
            if want == "exempt" and row.posture == "no-egress" and exempt:
                print(row.unit.relative_to(root))
            elif want == row.posture and not (row.posture == "no-egress" and exempt):
                print(row.unit.relative_to(root))
        return 0

    if "--explain" in argv:
        for row in rows:
            mark = "IPAddressDeny=any" if row.has else "-"
            extra = f"  +{len(row.dropins)} drop-in(s)" if row.dropins else ""
            print(f"{row.posture:14} {row.binary or '(no ExecStart)':26} "
                  f"{mark:18} {row.unit.relative_to(root)}{extra}")
        return 0

    # ---- the debt ratchet (#293) ------------------------------------
    # Checked before anything else, because both lists below SUPPRESS
    # findings: a commit that adds an entry is a commit that turns a
    # check off, and it may not do that in one green line. See
    # DEBT_CEILING's note for what this does and does not buy.
    for name, entries in (("EXEMPT", EXEMPT),
                          ("USER_SCOPE_INET_DEBT", USER_SCOPE_INET_DEBT),
                          ("DBUS_UNSANDBOXED_DEBT", DBUS_UNSANDBOXED_DEBT)):
        ceiling = DEBT_CEILING.get(name)
        if ceiling is None:
            errors.append(
                f"{name} has no entry in DEBT_CEILING. A list that removes "
                f"units from this gate must have a stated maximum, or it is a "
                f"convention again."
            )
        elif len(entries) > ceiling:
            errors.append(
                f"{name} holds {len(entries)} entr(ies) and DEBT_CEILING says "
                f"at most {ceiling}: {sorted(entries)}. Every entry here takes "
                f"a Lisa daemon OUT of a check — EXEMPT removes it from the "
                f"runtime suite as well, via `--list no-egress`. If the debt "
                f"is real, raise the ceiling in the same commit and say why: "
                f"that line is the one a reviewer has to read, which is the "
                f"whole point (#293)."
            )
        elif len(entries) < ceiling:
            errors.append(
                f"{name} holds {len(entries)} entr(ies) but DEBT_CEILING still "
                f"says {ceiling}. Lower it — a ceiling that outlives the debt "
                f"is headroom nobody decided to grant."
            )

    if not any(row.posture != "out-of-scope" for row in rows):
        # A matched-nothing sweep must fail, not pass: an installer
        # refactor that moves the install lines would otherwise turn
        # this into a green no-op — the defect class it polices.
        errors.append(
            "discovery found no unit running a Lisa binary at all — the "
            "installer scan is broken, not the tree clean"
        )

    covered_no_egress = set()
    covered_egress = set()
    for row in rows:
        rel = row.unit.relative_to(root)
        posture, binary, has = row.posture, row.binary, row.has

        # A wrapper between systemd and the daemon means the sandbox
        # this file reads is not the sandbox that process runs under —
        # and it used to file the daemon under "runs something else"
        # (#292). Refused outright rather than modelled: `ExecStart=`
        # naming the binary is one line, and every argument for the
        # wrapper is an argument for a script the unit could ship.
        if row.wrapped:
            errors.append(
                f"{rel}: reaches `{binary}` through a shell or `env` wrapper. "
                f"ExecStart must name the binary directly — a wrapper hides "
                f"the daemon from this gate (it read as an out-of-scope unit "
                f"running `sh`) and hides the real argv from the Ledger."
            )

        # IPAddressDeny= IS A NO-OP IN USER SCOPE. It is a cgroup BPF
        # program, and `systemd --user` cannot load one:
        #
        #   lisa-agentd.service: unit configures an IP firewall, but not
        #   running as root.
        #
        # — from the reference iMac's own journal. Measured there on
        # 2026-08-05 with two transient user units:
        #
        #   IPAddressDeny=any + RestrictAddressFamilies=AF_UNIX AF_INET
        #     -> curl http://example.com  HTTP=200   (reached the world)
        #   IPAddressDeny=any + RestrictAddressFamilies=AF_UNIX
        #     -> curl http://example.com  rc=7       (blocked)
        #
        # So for a user unit the ONLY directive that bites is
        # RestrictAddressFamilies, which is a seccomp filter and needs no
        # privilege. This check asserted IPAddressDeny alone and would
        # have passed forever over lisa-harnessd — the model host — and
        # over lisa-inferenced-dbus, whose "egress sandbox" commit
        # (47fa7c6) claimed a confinement it did not have.
        if posture == "no-egress" and row.user_scope and not row.inet \
                and row.unit.name in USER_SCOPE_INET_DEBT:
            errors.append(
                f"{rel}: no longer permits AF_INET and is still listed in "
                f"USER_SCOPE_INET_DEBT. Delete the entry — #288 is paid for "
                f"this unit, and a debt list that outlives the debt is how "
                f"the next reader learns to distrust it."
            )
        elif posture == "no-egress" and row.user_scope and row.inet \
                and row.unit.name not in USER_SCOPE_INET_DEBT:
            errors.append(
                f"{rel}: runs `{binary}` under `systemd --user`, where "
                f"{DIRECTIVE} is a NO-OP (an IP firewall needs root), and "
                f"still permits AF_INET. Nothing confines this unit. Give it "
                f"`RestrictAddressFamilies=AF_UNIX` — a seccomp filter, which "
                f"works unprivileged — or move the daemon off IP sockets. "
                f"Measured on hardware: the same directives with AF_INET "
                f"allowed reach example.com with HTTP 200."
            )
        if posture == "UNCLASSIFIED":
            errors.append(
                f"{rel}: runs `{binary}`, which has no egress posture. Add it "
                f"to NO_EGRESS or EGRESS in {Path(__file__).name} with the "
                f"reason — a daemon nobody classified is a daemon nobody "
                f"checked (CLAUDE.md rule 5)."
            )
        elif posture == "no-egress":
            if row.unit.name in EXEMPT:
                # NOT counted towards REQUIRED_NO_EGRESS_COVERAGE (#293).
                # It used to be: `covered_no_egress.add(binary)` ran
                # before this branch, so exempting lisa-agentd.service
                # left the "every rule-5 daemon is proven" floor
                # satisfied by the very unit the exemption had just
                # excused. A coverage floor that counts the units it
                # stopped checking is not a floor.
                if has:
                    errors.append(
                        f"{rel}: now carries {DIRECTIVE} and is still listed in "
                        f"EXEMPT. Delete the exemption — it exists to be "
                        f"deleted."
                    )
            else:
                covered_no_egress.add(binary)
                if not has:
                    errors.append(
                        f"{rel}: runs `{binary}`, which may never reach the "
                        f"network, and does not carry {DIRECTIVE}. The unit's "
                        f"comments are not a sandbox — "
                        f"lisa-inferenced-dbus.service shipped that way (#275)."
                    )
        elif posture == "egress":
            covered_egress.add(binary)
            if has:
                errors.append(
                    f"{rel}: runs `{binary}`, the egress broker, but carries "
                    f"{DIRECTIVE}. Either the broker cannot broker, or its "
                    f"posture in {Path(__file__).name} is wrong."
                )

    for binary in REQUIRED_NO_EGRESS_COVERAGE:
        if binary not in covered_no_egress:
            errors.append(
                f"CLAUDE.md rule 5 names `{binary}` as a no-egress daemon, but "
                f"discovery found no shipped unit running it — or every unit "
                f"running it is exempt. Either it stopped shipping, the "
                f"installer scan stopped seeing it, or an exemption excused "
                f"the last one; all three mean this check is no longer "
                f"covering it."
            )
    for binary in REQUIRED_EGRESS_COVERAGE:
        if binary not in covered_egress:
            errors.append(
                f"`{binary}` is the sole permitted egress path, and no shipped "
                f"unit runs it. 'Only remoted' has to be a statement about "
                f"something that exists."
            )

    if errors:
        print("egress posture check FAILED:\n", file=sys.stderr)
        for e in errors:
            print(f"  {e}\n", file=sys.stderr)
        return 1

    n = sum(1 for row in rows if row.posture == "no-egress")
    m = sum(1 for row in rows if row.posture == "egress")
    d = sum(len(row.dropins) for row in rows)
    print(f"egress posture: {n} no-egress units, {m} egress units, "
          f"{d} drop-in(s) merged, {len(EXEMPT)} exemption(s) — OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
