<script setup lang="ts">
// Portal home: compact hero, shipped strip, section entry cards, issues teaser.
const { loggedIn, user } = useUserSession()
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'Lisa OS — developer portal' })
</script>

<template>
  <div>
    <header class="hero">
      <span class="eyebrow">Developer portal</span>
      <h1 v-if="loggedIn">Welcome back, {{ user?.name }}.</h1>
      <h1 v-else>Lisa OS, for the people building on it.</h1>
      <p>An AI-native, self-updating Linux distribution: local models as a system service, per-app durable context, an append-only Ledger, and an OpenAI-compatible endpoint right on the machine. Flash it, boot it, build against it.</p>
      <div class="row">
        <NuxtLink class="btn solid" to="/docs/getting-started">Get started</NuxtLink>
        <NuxtLink class="btn line" to="/downloads">Latest release</NuxtLink>
      </div>
    </header>

    <section class="sec anchor">
      <h2>What shipped this week.</h2>
      <p>Summarised from <code>docs/STATUS.md</code>, which records what landed <em>and</em> what has not been seen on a screen yet. Some of the entries below are verified on the reference iMac and say so; others are CI facts. STATUS carries the caveat per item — read it there before relying on any of this.</p>
      <ul class="ship">
        <li><span class="d">Aug 5</span><span><strong>v20260805.81</strong> shipped and is running on the reference hardware: the image now records its own package manifest at <code>/usr/lib/lisa/packages.manifest</code>, so "which packages are in this image?" is answerable from the disk rather than the build log</span></li>
        <li><span class="d">Aug 5</span><span>The <strong>ports lane</strong> (ADR-0051): llama.cpp, whisper.cpp, piper, Zen and the Settings fork are built when their PKGBUILD changes and consumed from <code>os/packages/ports.lock</code> by sha256 — a release assembles, it no longer compiles pinned third-party code that has not changed</span></li>
        <li><span class="d">Aug 5</span><span>The fork family is <code>lisa-desktop-*</code>, and each package now <strong>replaces stock by <code>provides</code>/<code>conflicts</code></strong> rather than taking its name and winning on <code>pkgrel</code> — a race that silently loses the day Arch ships a higher version, which it did</span></li>
        <li><span class="d">Aug 5</span><span>A CI review across all four repos: ~30 findings, the mechanical ones fixed the same day. The dominant class was <em>gates that stayed green while asserting nothing</em> — a skipped branch printing nothing, a fallback examining the wrong artifact, a pipeline whose exit status belonged to the wrong command</span></li>
        <li><span class="d">Aug 3</span><span>The <code>[lisa]</code> pacman repo is live and <strong>signed</strong> — import the key, add the stanza at <code>SigLevel = Required</code> (<code>os/layer/install.sh</code> does both), and install <code>lisa-desktop</code>, <code>lisa-apps</code>, <code>lisa-cli</code> on any Arch machine</span></li>
        <li><span class="d">Aug 3</span><span>v20260802.63: the browser left the image (−363&nbsp;MiB) — Zen now updates from the apps channel in minutes, no reboot; real semantic search shipped (<code>nomic-embed-text-v1.5</code> behind <code>lisa context</code>, named in every Ledger row)</span></li>
        <li><span class="d">Aug 2</span><span>The split: <a href="https://github.com/Lisa-AgenticOS/lisa-desktop">lisa-desktop</a> (Shell surfaces + IME, becoming a hard fork of GNOME Shell — ADR-0038), <a href="https://github.com/Lisa-AgenticOS/lisa-apps">lisa-apps</a> (Mail, Surfer, Preview), <a href="https://github.com/Lisa-AgenticOS/lisa-packages">lisa-packages</a> (the index) — full history preserved, each building its own package (ADR-0039)</span></li>
        <li><span class="d">Aug 2</span><span>Design tokens: <code>branding/tokens.json</code> is the one source for every surface color, enforced by a lint gate — the fourth violet is now a red build, not a review comment</span></li>
        <li><span class="d">Aug 1</span><span>Preview app: images + PDF, Space-to-Quick-Look in Files (<code>NautilusPreviewer2</code>), MCP agent surface; Mail renders HTML, groups multi-account folders, and composes replies/forwards (RFC-correct threading)</span></li>
        <li><span class="d">Aug 2</span><span>Google OAuth brand verification approved — no more "unverified app" screen on Google sign-in</span></li>
      </ul>
      <p class="more"><NuxtLink to="/news">→ All news &amp; release notes</NuxtLink></p>
    </section>

    <section class="sec anchor">
      <h2>The system, in parts.</h2>
      <p>Each part is a way in. The numbering is the sheet's, not a ranking.</p>
      <div class="tilewrap"><div class="parts">
        <div class="part s5 r3">
          <span class="no">01 · Substrate</span>
          <p class="big">Local models as a system service</p>
          <p><code>lisa-inferenced</code> supervises a llama.cpp child and speaks an OpenAI-compatible API on the machine. Guided generation, a QoS scheduler that preempts background work, and embeddings — all behind one socket, and none of it reachable from the network.</p>
        </div>
        <div class="part s4 r3">
          <span class="no">02 · Record</span>
          <p class="big">An append-only Ledger</p>
          <p>Every model call is written before it runs: no entry, no action. UPDATE and DELETE are refused by trigger, not by convention.</p>
        </div>
        <div class="part s3 r3">
          <span class="no">03 · Egress</span>
          <p class="big">One audited door</p>
          <p><code>lisa-remoted</code> is the only daemon that may reach the internet. <code>lisa-contextd</code>, <code>lisa-agentd</code> and the app servers get <code>AF_UNIX</code> and nothing else; <code>lisa-inferenced</code> and <code>lisa-harnessd</code> are pinned to loopback because they serve HTTP on <code>127.0.0.1</code>. It is <code>IPAddressDeny=any</code> plus <code>RestrictAddressFamilies=</code> in each unit, and CI proves the sandbox blocks egress against a control that succeeds without it.</p>
        </div>
        <div class="part s4 r3">
          <span class="no">04 · Context</span>
          <p class="big">Per-app durable memory</p>
          <p><code>lisa-contextd</code> holds namespace-isolated context with provenance on every chunk, hybrid search over it, and a wipe that leaves no residue.</p>
        </div>
        <div class="part s4 r3">
          <span class="no">05 · Tools</span>
          <p class="big">Apps as agent surfaces</p>
          <p>Apps declare MCP tools; the Agent Bus tiers every call by blast radius and escalates on untrusted provenance — deterministic code, not prompt text.</p>
        </div>
        <div class="part s4 r3">
          <span class="no">06 · Delivery</span>
          <p class="big">A/B image, signed index</p>
          <p>Atomic updates with boot-counted rollback, plus <code>[lisa]</code> — a signed pacman repo you can point any Arch machine at.</p>
        </div>
        <NuxtLink class="part s3 r2" to="/docs"><span class="no">Sheet A</span><h3>Docs</h3><p>Install, architecture, building apps, the <code>lisa</code> CLI.</p></NuxtLink>
        <NuxtLink class="part s3 r2" to="/api"><span class="no">Sheet B</span><h3>API reference</h3><p>The HTTP endpoint, D-Bus interfaces and tool manifests — generated from source.</p></NuxtLink>
        <NuxtLink class="part s3 r2" to="/downloads"><span class="no">Sheet C</span><h3>Downloads</h3><p>Every release and its assets, with checksums.</p></NuxtLink>
        <NuxtLink class="part s3 r2" to="/design"><span class="no">Sheet D</span><h3>Design</h3><p>The token source every surface reads, enforced by a lint gate.</p></NuxtLink>
      </div></div>
    </section>

    <section class="sec anchor">
      <h2>Start contributing.</h2>
      <p>Good first issues, straight from the repo. Pick one, comment to claim it, and open a PR.</p>
      <IssuesBoard :limit="3" />
      <p class="more"><NuxtLink to="/contribute">→ The full board, and how to get set up</NuxtLink></p>
    </section>

    <section class="sec anchor">
      <h2>Learn</h2>
      <p>The design is the source of truth, and every non-obvious decision is written down.</p>
      <div class="grid">
        <NuxtLink class="card" to="/docs/architecture"><h3>Architecture</h3><p>The daemon map, the portal boundary, the Ledger, the egress rules.</p></NuxtLink>
        <a class="card" :href="`${repo}/blob/main/docs/ROADMAP.md`"><h3>Roadmap</h3><p>What's built, what's next, milestone by milestone.</p></a>
        <a class="card" :href="`${repo}/tree/main/docs/adr`"><h3>Decisions (ADRs)</h3><p>Why the OS is built the way it is.</p></a>
        <a class="card" href="https://github.com/Lisa-AgenticOS"><h3>Source</h3><p>Four repos: lisa-os (the OS), lisa-desktop, lisa-apps, lisa-packages (the signed [lisa] index).</p></a>
      </div>
    </section>
  </div>
</template>
