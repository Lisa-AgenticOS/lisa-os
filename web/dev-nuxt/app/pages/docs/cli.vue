<script setup lang="ts">
// Generated from the clap definitions in cli/lisa/src/main.rs —
// descriptions are the actual doc comments, abridged.
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'CLI reference — Lisa OS developers' })

const verbs = [
  ['ask', 'Ask the system model. Reads stdin when piped, e.g. git log | lisa ask "changelog, markdown".', '--model, --no-stream, --json-schema <file>, --background, --url'],
  ['explain', 'Explain the last failed command: lisa explain --exit 101 cargo build, or pipe the output — make 2>&1 | lisa explain. Bare lisa explain uses what the terminal hooks stashed about the last failure.', '--exit <code>, --model, --url'],
  ['suggest', 'Natural language → ONE shell command, printed for review — never executed (the Ctrl+G terminal hook calls this). stdout is exactly the command; the explanation goes to stderr.', '--json, --model, --url'],
  ['models', 'Manage the local model store: list, verify, gc, rm, pull (pinned blake3), profile, catalog, get, hash, add.', '--store <dir>; per-subcommand flags like --json, --runnable, --blake3'],
  ['do', 'Natural language → a confirmed tool action on the Agent Bus: lisa do "add milk to my notes" (ADR-0013 intent router).', '--dry-run, --yes, --model, --url'],
  ['tools', 'List tools on the Agent Bus.', '—'],
  ['call', 'Call a tool on the Agent Bus directly: lisa call app.lisaos.notes create_note \'{"title":"milk"}\'.', '--yes'],
  ['undo', 'Revert the last undoable agent action.', '—'],
  ['ledger', 'Read the append-only audit ledger.', '--tail <n> (default 20), --json, --db <path>'],
  ['context', 'Context fabric: index and search your files. Subcommands: index <dir>, search <query>.', 'index: --embed · search: --limit, --hybrid, --scope (repeatable)'],
  ['memory', 'Per-app durable memory. Subcommands: get, set, list, wipe.', '--app <namespace> (default "host"); wipe: --yes'],
  ['install', 'Write the newest Lisa OS release to a whole disk — ERASES IT. The proto-installer (a guided OOBE installer is M7).', '--from <file>, --yes'],
  ['update', 'Pull the newest OS release into the inactive A/B slot (systemd-sysupdate; Track I systems).', '--reboot'],
  ['apps', 'Update the shell apps independently of the OS image (ADR-0020). Subcommands: update, status, rollback.', '—'],
  ['remote', 'Manage BYO remote model providers. Subcommands: list, add, key, consent. Inference uses them via lisa ask --model remote:<provider>:<model>.', 'consent <scope> <on|off> — scopes: prompt, files, mail, calendar, screen, memory'],
  ['transcribe', 'Transcribe an audio file with whisper.cpp (STT).', '--model <path>'],
  ['say', 'Speak text with the local voice (piper / say) (TTS).', '—'],
  ['ambient', 'Lisa Ambient: the voice loop (ADR-0011). Subcommands: classify, once.', 'once: --speak, --classify, --model, --url'],
  ['forge', 'LisaCode: talk an app into existence — the Forge harness drives a model to write + fix code until it passes analysis.', '--project <dir> (default ./lisa-app), --model, --max-iters <n> (default 6), --url'],
  ['embed', 'Embed text into a vector (reads stdin when piped).', '--url']
]
</script>

<template>
  <div>
    <span class="eyebrow">Docs / CLI reference</span>
    <h1>The <code>lisa</code> command center.</h1>
    <p class="lede">One command center: every user-facing verb lives under <code>lisa &lt;verb&gt;</code> — never scattered helper scripts (PLAN §5.4, Appendix E rule 4). This table is generated from the actual clap definitions in <a :href="`${repo}/blob/main/cli/lisa/src/main.rs`">cli/lisa/src/main.rs</a>.</p>
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
