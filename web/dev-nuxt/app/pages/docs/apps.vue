<script setup lang="ts">
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'Building apps — Lisa OS developers' })
</script>

<template>
  <div>
    <span class="eyebrow">Docs / Building apps</span>
    <h1>Apps on Lisa.</h1>
    <p class="lede">One toolkit: <strong>GJS + GTK4/Adwaita</strong> (ADR-0047). The shell surfaces that ship in the image (Assistant, Ledger app, Settings, the overlay backend under <code>shell/</code>) and the apps (Mail, Surfer, Preview) are the same shape, so a fix reaches a device by copying files. Apps expose their actions to the system as MCP tools; the Agent Bus enforces confirmation tiers so apps don't have to.</p>

    <h2>The <code>lisa-app</code> launcher</h2>
    <p>Shell apps are interpreted (GJS), so updating them is copying files. <code>/usr/bin/lisa-app &lt;relpath&gt;</code> execs <code>gjs -m</code> on the app entry point, resolved out of the current apps tree. It does not know where that tree is: it asks <code>lisa apps path shell</code>, the same function <code>lisa apps update</code> installs through, so the writer and the reader cannot disagree (issue #239). <code>.desktop</code> files and D-Bus activation exec via <code>lisa-app</code>, so an updated tree takes effect on the next app launch, no reboot (ADR-0020).</p>

    <h2>The app update channel</h2>
    <p>App updates are decoupled from the OS image (ADR-0020): a versioned apps tree lives on the persistent <code>/var</code>, flipped atomically via symlink+rename — no partial states.</p>
    <pre><code>lisa apps update    <span class="c"># fetch lisa-apps_&lt;ver&gt;.tar.zst, verify vs SHA256SUMS, unpack, flip</span>
lisa apps status    <span class="c"># installed versions AND the directory a launch would actually use</span>
lisa apps rollback  <span class="c"># flip back to the previous tree (or the baked image tree)</span>
lisa apps path shell<span class="c"># the directories a launcher searches, best first</span></code></pre>
    <ul>
      <li>Same GitHub Releases channel and manifest the OS updates use — one release, two update planes.</li>
      <li>The tree carries the shell surfaces <em>and</em> the apps (Mail, Surfer, Preview) — the same tree the image bakes at <code>/usr/share/lisa/shell</code>, staged by the same script.</li>
      <li>A broken tree is one <code>lisa apps rollback</code> away; with nothing older installed that restores the baked image tree.</li>
      <li>No <code>sudo</code>: the payload directories are group-writable for the desktop user (ADR-0034 §7b).</li>
      <li>GNOME Shell <em>extensions</em> load at session start from the baked tree — they're out of scope for this channel and keep riding image releases.</li>
      <li>This channel is the interim. The intent is Flatpak-first for GUI apps, because the portal is the security boundary — but <strong>nothing Lisa ships is a Flatpak today</strong>, and these apps launch unsandboxed (PLAN §5.8). The portal and <code>lisa-peer</code> already resolve a Flatpak app-id if one appears; nothing produces one yet.</li>
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

    <h2>Checking an app: <code>lisa dev check</code></h2>
    <p>One command decides whether a directory is a valid Lisa app (ADR-0050), and it is the same judgement the Forge uses as its verifier — so generated code and hand-written code are held to one standard. It gates on there being source at all, on no top-level <code>await</code> in an entry module (the failure that binds a socket, advertises it and answers nothing, with no error in any log), and on the manifest, parsed by the same code <code>lisa-agentd</code> runs.</p>
    <pre><code>lisa dev check apps/notes   <span class="c"># prints findings and exits non-zero if there are any</span>
lisa forge "a notes app" --project ~/notes</code></pre>
    <p>It deliberately does not run the app's own tests and makes no JavaScript syntax claim — the verifier runs unconfined, and a checker that executes model-written code in order to verify it would hand the loop the escape the jail exists to prevent.</p>

    <h2>The Flutter lane is parked</h2>
    <p>The Flutter lane is gone: the Dart kits were deleted on 2026-08-06 and the whole lane — SDK installer, forge flags, guard rules — was removed on 2026-08-07 (ADR-0047 amendment). Nothing user-facing was ever written in Flutter; the reasons GJS won were iteration on real hardware, desktop integration (portals, D-Bus activation, AT-SPI, input methods) that GTK4 gets for free, and one toolkit meaning one design-token sheet, one test harness and one set of idioms. The shared library every app builds on is <code>apps/lisa.sdk</code>.</p>

    <h2>No SDK required</h2>
    <p>You don't need anything Lisa-specific to build against the intelligence: the OpenAI-compatible endpoint on <code>127.0.0.1:7777</code> works with any existing OpenAI client — Electron, web, CLI, anything. Guided generation (<code>response_format: json_schema</code>) gives you typed output that always parses. See <NuxtLink to="/api">the API reference</NuxtLink> and <a :href="`${repo}/tree/main/docs/sdk/samples`">docs/sdk/samples</a>.</p>
  </div>
</template>
