<script setup lang="ts">
// Transcribed by hand from the clap definitions in cli/lisa/src/main.rs
// — descriptions are the actual doc comments, abridged. Keep this in
// declaration order and complete: a verb table that silently omits verbs
// (guard, assist, mail, doctor, listen, dev, skills, completions were all
// missing on 2026-08-05) reads as "these are the verbs there are".
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'CLI reference — Lisa OS developers' })

// In declaration order, so the table and the enum can be diffed by eye.
const verbs = [
  ['ask', 'Ask the system model. Reads stdin when piped, e.g. git log | lisa ask "changelog, markdown".', '--model, --no-stream, --json-schema <file>, --background, --url'],
  ['explain', 'Explain the last failed command: lisa explain --exit 101 cargo build, or pipe the output — make 2>&1 | lisa explain. Bare lisa explain uses what the terminal hooks stashed about the last failure.', '--exit <code>, --model, --url'],
  ['suggest', 'Natural language → ONE shell command, printed for review — never executed (the Ctrl+G terminal hook calls this). stdout is exactly the command; the explanation goes to stderr.', '--json, --model, --url'],
  ['guard', 'Inspect and relax the action guard (ADR-0029, ADR-0030) — the OUTSIDE of the boundary: you set policy for the model on your machine, and nothing the agent can invoke reaches it. Subcommands: list, allow <rule>, forbid <rule>.', '—'],
  ['models', 'Manage the local model store: list, verify, gc, rm, pull (pinned blake3), profile, catalog, get, hash, add.', '--store <dir>; per-subcommand flags like --json, --runnable, --blake3'],
  ['do', 'Natural language → a confirmed tool action on the Agent Bus: lisa do "add milk to my notes" (ADR-0013 intent router).', '--dry-run, --yes, --model, --url'],
  ['assist', 'Multi-turn assistant on the agent harness: read-tier Agent Bus tools in one loop (ADR-0025). Unlike lisa do, which routes one utterance to one tool call, this can look, read the answer, and look again.', '--model, --max-turns <n> (default 12), --url'],
  ['tools', 'List tools on the Agent Bus.', '—'],
  ['call', 'Call a tool on the Agent Bus directly: lisa call app.lisaos.notes create_note \'{"title":"milk"}\'.', '--yes'],
  ['undo', 'Revert the last undoable agent action.', '—'],
  ['ledger', 'Read the append-only audit ledger.', '--tail <n> (default 20), --json, --db <path>'],
  ['context', 'Context fabric: index and search your files. Subcommands: index <dir>, search <query>, sync-knowledge.', 'index: --embed · search: --limit, --hybrid, --scope (repeatable)'],
  ['memory', 'Per-app durable memory. Subcommands: get, set, list, wipe.', '--app <namespace> (default "host"); wipe: --yes'],
  ['install', 'Write the newest Lisa OS release to a whole disk — ERASES IT. The proto-installer (a guided OOBE installer is M7).', '--from <file>, --yes'],
  ['update', 'Pull the newest OS release into the inactive A/B slot (systemd-sysupdate; Track I systems).', '--reboot'],
  ['mail', 'Join a connected mail account to the Maildir the Mail app reads (#155). Lisa does not speak IMAP itself — this writes the mbsync config and fetches the token mbsync authenticates with. Subcommands: setup, sync, index, status, token.', '—'],
  ['doctor', 'Collect the state of this machine into one shareable report: versions, services, storage, desktop, the Lisa units\' warnings, the Ledger tail. Credentials are removed and prompt text is withheld unless you ask for it.', '--bundle [<file>], --include-previews, --journal-lines <n> (default 200)'],
  ['apps', 'Update out-of-image payloads independently of the OS image (ADR-0020, ADR-0023). Subcommands: update, status, rollback, sync, install, path.', '—'],
  ['remote', 'Manage BYO remote model providers. Subcommands: list, add, key, consent. Inference uses them via lisa ask --model remote:<provider>:<model>.', 'consent <scope> <on|off> — scopes: prompt, files, mail, calendar, screen, memory'],
  ['transcribe', 'Transcribe an audio file with whisper.cpp (STT).', '--model <path>'],
  ['listen', 'Record from the microphone and transcribe it (push-to-talk) — the capture half every other voice verb assumed.', '--seconds <n> (default 15, a ceiling), --model, --keep <file>'],
  ['say', 'Speak text with the local voice (piper / say) (TTS).', '—'],
  ['ambient', 'Lisa Ambient: the voice loop (ADR-0011). Subcommands: classify, once.', 'once: --speak, --classify, --model, --url'],
  ['forge', 'LisaCode: talk an app into existence — the Forge harness drives a model to write + fix code until it passes analysis. The lane is GJS, verified by lisa dev check (ADR-0047).', '--project <dir> (default ./lisa-app), --model, --max-iters <n> (default 6), --url'],
  ['dev', 'Developer tooling for building on Lisa. check <path> is the single authority on what a valid Lisa app is (ADR-0050) and is the Forge\'s verifier; install / remove / list / shell / reset / doctor are the rootless dev box in your home (ADR-0034).', 'doctor: --needs <GiB>'],
  ['skills', 'Skills: the SKILL.md workflows Lisa loads on demand (ADR-0025). Subcommands: list, show <name>.', '—'],
  ['embed', 'Embed text into a vector (reads stdin when piped).', '--url'],
  ['completions', 'Print a shell completion script to stdout, e.g. lisa completions zsh > ~/.zfunc/_lisa. Packages install these at the standard paths.', '<shell>: bash, zsh, fish, elvish, powershell']
]
</script>

<template>
  <div>
    <span class="eyebrow">Docs / CLI reference</span>
    <h1>The <code>lisa</code> command center.</h1>
    <p class="lede">One command center: every user-facing verb lives under <code>lisa &lt;verb&gt;</code> — never scattered helper scripts (PLAN §5.4, Appendix E rule 4). Every verb below is transcribed from the clap definitions in <a :href="`${repo}/blob/main/cli/lisa/src/main.rs`">cli/lisa/src/main.rs</a>, in declaration order — descriptions are the doc comments, abridged. It is a copy, not a generator, so <code>lisa --help</code> on your machine is the authority.</p>
    <p>Text verbs share one endpoint flag: <code>--url</code> defaults to <code>http://127.0.0.1:7777</code> (env <code>LISA_INFERENCE_URL</code>).</p>

    <div class="tbl"><table>
      <thead><tr><th>Verb</th><th>What it does</th><th>Key flags</th></tr></thead>
      <tbody>
        <tr v-for="[v, desc, flags] in verbs" :key="v">
          <td><code>lisa {{ v }}</code></td>
          <td>{{ desc }}</td>
          <td>{{ flags }}</td>
        </tr>
      </tbody>
    </table></div>

    <h2>Pipes are context</h2>
    <pre><code>git log | lisa ask "changelog, markdown"   <span class="c"># piped stdin becomes context</span>
make 2&gt;&amp;1 | lisa explain                   <span class="c"># explain a failing build</span>
lisa ask --json-schema recipe.json "…"     <span class="c"># grammar-constrained: always parses</span>
lisa ask --model remote:moonshot:kimi-k2 … <span class="c"># BYO cloud via the egress broker, ledgered</span></code></pre>
    <p>Remote (<code>remote:&lt;provider&gt;:&lt;model&gt;</code>) requests go to the egress broker, not the local engine: the broker holds the key, enforces per-scope consent, and ledgers the egress — the inference daemon never gets network.</p>

    <h2>Safety posture, encoded in the verbs</h2>
    <ul>
      <li><code>lisa suggest</code> prints a command for review — it <strong>never executes</strong>; the Ctrl+G hook inserts it for you to edit before Enter.</li>
      <li><code>lisa install</code> demands a typed confirmation before touching a disk (<code>--yes</code> is for scripts/CI only).</li>
      <li><code>lisa do</code> and <code>lisa call</code> honor the bus's confirmation tiers; <code>--yes</code> auto-approves chip-level confirmations only — modal ones still refuse.</li>
      <li>Model output is sanitized before printing: terminal escape sequences from untrusted text can't reach your terminal.</li>
    </ul>
  </div>
</template>
