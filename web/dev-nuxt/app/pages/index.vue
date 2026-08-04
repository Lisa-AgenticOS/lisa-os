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
      <p>From <code>docs/STATUS.md</code> — verified on real hardware, not aspirational.</p>
      <ul class="ship">
        <li><span class="d">Aug 3</span><span>The <code>[lisa]</code> pacman repo is live and <strong>signed</strong> — add one <code>Server=</code> line and install <code>lisa-desktop</code>, <code>lisa-apps</code>, <code>lisa-cli</code> on any Arch machine</span></li>
        <li><span class="d">Aug 3</span><span>v20260802.63: the browser left the image (−363&nbsp;MiB) — Zen now updates from the apps channel in minutes, no reboot; real semantic search shipped (<code>nomic-embed-text-v1.5</code> behind <code>lisa context</code>, named in every Ledger row)</span></li>
        <li><span class="d">Aug 2</span><span>The split: <a href="https://github.com/Lisa-AgenticOS/lisa-desktop">lisa-desktop</a> (Shell surfaces + IME, becoming a hard fork of GNOME Shell — ADR-0038), <a href="https://github.com/Lisa-AgenticOS/lisa-apps">lisa-apps</a> (Mail, Surfer, Preview), <a href="https://github.com/Lisa-AgenticOS/lisa-packages">lisa-packages</a> (the index) — full history preserved, each building its own package (ADR-0039)</span></li>
        <li><span class="d">Aug 2</span><span>Design tokens: <code>branding/tokens.json</code> is the one source for every surface color, enforced by a lint gate — the fourth violet is now a red build, not a review comment</span></li>
        <li><span class="d">Aug 1</span><span>Preview app: images + PDF, Space-to-Quick-Look in Files (<code>NautilusPreviewer2</code>), MCP agent surface; Mail renders HTML, groups multi-account folders, and composes replies/forwards (RFC-correct threading)</span></li>
        <li><span class="d">Aug 2</span><span>Google OAuth brand verification approved — no more "unverified app" screen on Google sign-in</span></li>
      </ul>
      <p class="more"><NuxtLink to="/news">→ All news &amp; release notes</NuxtLink></p>
    </section>

    <section class="sec anchor">
      <h2>Explore.</h2>
      <div class="grid">
        <NuxtLink class="card" to="/docs"><h3>Docs</h3><p>Install the OS, understand the architecture, build and ship apps, master the <code>lisa</code> CLI.</p></NuxtLink>
        <NuxtLink class="card" to="/api"><h3>API reference</h3><p>The OpenAI-compatible HTTP endpoint, the D-Bus interfaces, and MCP tool manifests — from source.</p></NuxtLink>
        <NuxtLink class="card" to="/downloads"><h3>Downloads</h3><p>Every release with its assets — USB image, sysupdate transfer set, apps tarball, checksums.</p></NuxtLink>
        <NuxtLink class="card" to="/design"><h3>Design</h3><p>The violet ramp, editorial tokens, Rubik, and the one token source every surface reads.</p></NuxtLink>
        <NuxtLink class="card" to="/news"><h3>News</h3><p>What shipped, week by week, plus release notes straight from GitHub.</p></NuxtLink>
        <NuxtLink class="card" to="/contribute"><h3>Contribute</h3><p>Good first issues, live from the repo. Claim one, open a PR.</p></NuxtLink>
      </div>
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
