<script setup lang="ts">
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'Building apps — Lisa OS developers' })
</script>

<template>
  <div>
    <span class="eyebrow">Docs / Building apps</span>
    <h1>Apps on Lisa.</h1>
    <p class="lede">Two lanes today: the GJS shell surfaces that ship in the image (Assistant, Ledger app, Settings, the overlay backend under <code>shell/</code>), and the Flutter lane built on <code>lisa_ui</code>. Apps expose their actions to the system as MCP tools; the Agent Bus enforces confirmation tiers so apps don't have to.</p>

    <h2>The <code>lisa-app</code> launcher</h2>
    <p>Shell apps are interpreted (GJS), so updating them is copying files. <code>/usr/bin/lisa-app &lt;relpath&gt;</code> resolves the apps tree — <code>$LISA_APPS_DIR</code> → <code>/var/lib/lisa/apps/current</code> → the baked <code>/usr/share/lisa/shell</code> fallback — and execs <code>gjs -m</code> on the app entry point. <code>.desktop</code> files and D-Bus activation exec via <code>lisa-app</code>, so an updated tree takes effect on the next app launch, no reboot (ADR-0020).</p>

    <h2>The app update channel</h2>
    <p>App updates are decoupled from the OS image (ADR-0020): a versioned apps tree lives on the persistent <code>/var</code>, flipped atomically via symlink+rename — no partial states.</p>
    <pre><code>lisa apps update    <span class="c"># fetch lisa-apps_&lt;ver&gt;.tar.zst, verify vs SHA256SUMS, unpack, flip</span>
lisa apps status    <span class="c"># current and installed apps-tree versions</span>
lisa apps rollback  <span class="c"># flip back to the previous tree (or the baked image tree)</span></code></pre>
    <ul>
      <li>Same GitHub Releases channel and manifest the OS updates use — one release, two update planes.</li>
      <li>A broken tree is one <code>lisa apps rollback</code> away; deleting <code>/var/lib/lisa/apps</code> always restores the baked tree.</li>
      <li>GNOME Shell <em>extensions</em> load at session start from the baked tree — they're out of scope for this channel and keep riding image releases.</li>
      <li>This channel is the interim: it's superseded by the Flatpak lane when M6 matures.</li>
    </ul>

    <h2>GJS shell surfaces</h2>
    <p>The first-party surfaces are TypeScript/GJS under <code>shell/</code>:</p>
    <ul>
      <li><strong>Assistant</strong> (<code>shell/assistant</code>, ADR-0015) — a persistent chat window, a frontend of the overlay backend: local + cloud models, streaming, ledgered. Super+C opens it.</li>
      <li><strong>Overlay</strong> (<code>shell/overlay-extension</code>) — one headless backend owning state/streams (<code>dev.lisaos.Overlay1</code>) with thin frontends; Super+Shift+Space summons it.</li>
      <li><strong>Launcher</strong> (<code>shell/launcher</code>) — the semantic search provider with the "Ask Lisa" handoff.</li>
      <li><strong>Ledger app</strong> (<code>shell/ledger-app</code>, GTK4/GJS) — renders the audit DB.</li>
      <li><strong>Settings</strong> (<code>shell/settings</code>) — providers, consent, models.</li>
    </ul>
    <p>House style: fail-soft D-Bus calls everywhere — apps must degrade gracefully against older daemons, because the apps tree can be newer than the image.</p>

    <h2>Exposing tools (MCP manifests)</h2>
    <p>An app declares its actions in a manifest: typed tools with a JSON Schema per input, a confirmation <code>tier</code> (<code>read</code> / <code>write</code> / <code>destructive</code>), and optional <code>undo</code> mappings the bus journals for <code>lisa undo</code>. The Notes app (<code>apps/notes</code>) is the worked example — see the <NuxtLink to="/api#mcp">manifest walkthrough in the API reference</NuxtLink>. Tiers are enforced at the bus, not by app goodwill.</p>

    <h2>The Flutter lane and <code>lisa_ui</code></h2>
    <p><code>libs/lisa_ui</code> is the widget kit Lisa apps import (ADR-0014, phase 1): a curated re-export of the Material vocabulary plus Lisa-branded widgets and theming, behind a single <code>import 'package:lisa_ui/lisa_ui.dart'</code>. Violet seed color, Material 3, Rubik (OS-installed, platform sans fallback). Phase 2 swaps the backend to a vendored, re-themed Material fork with no app-facing API change.</p>
    <p><code>libs/lisa_flutter</code> is the zero-dependency Dart transport for the OpenAI-compatible endpoint, with a live round trip against the daemon.</p>
    <div class="note">Honest status: the kit is ready and widget-tested (see the <NuxtLink to="/design">Design page</NuxtLink> for the widget inventory), but the Flutter <em>app lane</em> — Forge-generated apps, Flatpak packaging, capability manifests — is milestone M6 and still pending. Notes and Recorder are slated to be built in this lane as permanent dogfood (PLAN §5.8).</div>

    <h2>No SDK required</h2>
    <p>You don't need anything Lisa-specific to build against the intelligence: the OpenAI-compatible endpoint on <code>127.0.0.1:7777</code> works with any existing OpenAI client — Electron, web, CLI, anything. Guided generation (<code>response_format: json_schema</code>) gives you typed output that always parses. See <NuxtLink to="/api">the API reference</NuxtLink> and <a :href="`${repo}/tree/main/docs/sdk/samples`">docs/sdk/samples</a>.</p>
  </div>
</template>
