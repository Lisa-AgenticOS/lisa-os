<script setup lang="ts">
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
const adr = (f: string) => `${repo}/blob/main/docs/adr/${f}`
useHead({ title: 'Architecture — Lisa OS developers' })

// Real ADRs, titles from the files in docs/adr/.
const adrs = [
  ['0001', 'arch-immutable-base', 'Fork Arch Linux; ship an immutable, atomic, image-based OS via mkosi'],
  ['0002', 'rust-zbus-axum', 'Rust with zbus + axum for system daemons'],
  ['0003', 'two-track-delivery', 'Two-track delivery — Lisa Layer first, immutable image as the product'],
  ['0004', 'flutter-lane-forge', 'Flutter app lane + the Forge'],
  ['0005', 'gpl2-license', 'License the project GPL-2.0-only'],
  ['0006', 'monorepo-staged-extraction', 'Monorepo with staged extraction'],
  ['0007', 'fcitx5-addon-cxx', 'fcitx5-lisa is a C++ addon (thin), logic stays on the daemon side'],
  ['0008', 'portal-standalone-service', 'The Lisa portal is a standalone session service, consent stays in the shell'],
  ['0009', 'agent-bus-core', 'Agent Bus core — D-Bus surface, tier enforcement at the bus, staged MCP transport'],
  ['0010', 'remote-providers', 'BYO remote model providers via a dedicated egress broker (lisa-remoted)'],
  ['0011', 'ambient-assistant', 'Lisa Ambient — the always-on, wake-word-free assistant'],
  ['0012', 'gnome-control-center-lisa-panel', 'A native "Intelligence" panel in a forked gnome-control-center'],
  ['0013', 'harness-intents-and-coding-agent', 'The Lisa harness — Siri-style intents + a Claude-Code-level coding agent, on the existing substrate'],
  ['0014', 'lisa-ui-material-fork', 'lisa_ui becomes the kit Lisa apps import — Material-backed now, vendored fork later'],
  ['0015', 'assistant-app', 'A persistent Assistant chat window — the surface that makes the model usable'],
  ['0016', 'reverse-dns-naming', 'Reverse-DNS identifiers move to the real domains (dev.lisaos.* / app.lisaos.*)'],
  ['0017', 'plymouth-in-initrd', 'Plymouth + the lisa theme move into the mkosi-initrd'],
  ['0018', 'var-pinned-partuuid', '/var is mounted by partition LABEL, not by UUID'],
  ['0019', 'dedicated-home-partition', 'A dedicated /home partition on fresh installs, weight-split with var'],
  ['0020', 'app-update-channel', 'App updates decoupled from the OS image']
]
</script>

<template>
  <div>
    <span class="eyebrow">Docs / Architecture</span>
    <h1>How Lisa is put together.</h1>
    <p class="lede">Distilled from <a :href="`${repo}/blob/main/docs/PLAN.md`">docs/PLAN.md</a> §4–§5. The shape in one sentence: shell surfaces and apps talk through a portal trust boundary to a small set of single-purpose daemons, everything is recorded in an append-only Ledger, and the daemons that hold your data have no network access.</p>

    <h2>The daemons</h2>
    <div class="tbl"><table>
      <thead><tr><th>Daemon</th><th>Role</th><th>Spec</th></tr></thead>
      <tbody>
        <tr><td><code>lisa-inferenced</code></td><td>Model runtime &amp; scheduler — the one process that owns compute. Supervises engine children (llama.cpp et al.), arbitrates RAM/VRAM, QoS scheduling (interactive preempts background), guided generation (JSON Schema → GBNF grammar). Exposes D-Bus <code>dev.lisaos.Inference1</code> and OpenAI-compatible HTTP on <code>127.0.0.1:7777</code>.</td><td>PLAN §5.1</td></tr>
        <tr><td><code>lisa-modeld</code></td><td>Model catalog &amp; store — blake3 content-addressed store, pinned-hash downloads, hardware profiler. The only component allowed network access for model traffic.</td><td>PLAN §5.2</td></tr>
        <tr><td><code>lisa-contextd</code></td><td>Context fabric — the personal context index (SQLite FTS5 + vectors) with provenance tags and per-chunk ACLs, plus namespace-isolated per-app durable memory. D-Bus <code>dev.lisaos.Context1</code>.</td><td>PLAN §5.3</td></tr>
        <tr><td><code>lisa-agentd</code></td><td>Agent Bus — apps are MCP servers; the bus holds the tool registry, enforces confirmation tiers (read → silent, write → chip, destructive → modal) at the bus, keeps the undo journal. D-Bus <code>dev.lisaos.Agent1</code>.</td><td>PLAN §5.4, ADR-0009</td></tr>
        <tr><td><code>lisa-remoted</code></td><td>Egress broker — the single ledgered path to BYO remote model providers, with per-scope offload consent (default: nothing leaves). D-Bus <code>dev.lisaos.Remote1</code>.</td><td>ADR-0010, PLAN §5.11</td></tr>
      </tbody>
    </table></div>

    <h2>The portal boundary</h2>
    <p><code>xdg-desktop-portal-lisa</code> (PLAN §5.5, ADR-0008) is the trust boundary: sandboxed apps never talk to daemons directly. The portal attaches per-app identity, runs first-use consent (fail-closed), enforces quotas, and writes Ledger attribution. Interfaces: <code>dev.lisaos.portal.{Inference, Context, Memory, Agent}</code>.</p>
    <div class="note">Status: the portal core is landed as a standalone session service — per-app identity, append-only grant store, revoke-kills-live-session — tested end-to-end against <code>dev.lisaos.Inference1</code>. The shell consent dialog and Settings surface are still open (M2/M4 work; see <code>docs/STATUS.md</code>).</div>

    <h2>The Ledger</h2>
    <p>An append-only SQLite audit log (<code>libs/lisa-ledger</code>) where UPDATE and DELETE are aborted by triggers. It is enforced, not advisory — dataflow rule 4: <strong>the ledger entry precedes the action; no ledger entry, no inference</strong>. Append failure returns 503, and the inference daemon refuses to start without a ledger. Context searches, tool calls, and remote egress are ledgered the same way. Read it with <code>lisa ledger</code> or the first-party Ledger app.</p>

    <h2>Egress rules</h2>
    <p>Egress is architecture, not policy (PLAN §5.10):</p>
    <ul>
      <li><code>lisa-inferenced</code>, <code>lisa-contextd</code>, and <code>lisa-agentd</code> run with <strong>no network access</strong> (systemd sandboxing; verified zero egress in CI).</li>
      <li>Only <code>lisa-modeld</code> gets network for model downloads, and only <code>lisa-remoted</code> brokers traffic to remote providers — every remote request is ledgered with a <code>remote.*</code> kind and rendered in the dedicated egress color.</li>
      <li>Per-scope offload consent (<code>prompt</code>, <code>files</code>, <code>mail</code>, <code>calendar</code>, <code>screen</code>, <code>memory</code>) defaults to <strong>off</strong> — even prompts don't leave until you say so.</li>
    </ul>

    <h2>Provenance</h2>
    <p>Every context chunk carries a provenance tag (<code>user</code>, <code>app:&lt;id&gt;</code>, <code>file</code>, <code>screen</code>, <code>web</code>). Untrusted-provenance content is data, never instructions: a privileged tool call whose trigger chain includes untrusted provenance escalates one confirmation tier, fail-closed. A seeded injection suite gates merges on zero unconfirmed privileged calls.</p>

    <h2>Decisions (ADRs)</h2>
    <p>Every non-obvious decision is written down in <a :href="`${repo}/tree/main/docs/adr`">docs/adr/</a>:</p>
    <div class="tbl"><table>
      <thead><tr><th>ADR</th><th>Title</th></tr></thead>
      <tbody>
        <tr v-for="[n, slug, title] in adrs" :key="n">
          <td><a :href="adr(`${n}-${slug}.md`)">ADR-{{ n }}</a></td>
          <td>{{ title }}</td>
        </tr>
      </tbody>
    </table></div>
  </div>
</template>
