<script setup lang="ts">
// News: the shipped-this-week strip (docs/STATUS.md) + live release notes.
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
const { data: releases } = await useFetch('/api/releases', { default: () => [] })
const fmtDate = (d: string) => (d ? d.slice(0, 10) : '')
useHead({ title: 'News — Lisa OS developers' })
</script>

<template>
  <div>
    <header class="pagehead">
      <span class="eyebrow">News</span>
      <h1>What shipped.</h1>
      <p class="lede">The weekly strip comes from <code>docs/STATUS.md</code> — verified on real hardware, not aspirational. Release notes are pulled live from GitHub, unedited.</p>
    </header>

    <section class="sec anchor">
      <h2>This week</h2>
      <ul class="ship">
        <li><span class="d">Jul 25</span><span>Assistant chat verified end-to-end on the field iMac — streamed, ledgered, live model picker; Stop, Markdown export, history across restarts</span></li>
        <li><span class="d">Jul 25</span><span>Terminal integration: <code>lisa explain</code> / <code>lisa suggest</code> — Ctrl+G with review-before-Enter; suggestions never auto-run</span></li>
        <li><span class="d">Jul 25</span><span><code>lisa apps update</code>: app updates in minutes, no reboot, atomic rollback (ADR-0020)</span></li>
        <li><span class="d">Jul 25</span><span>True token streaming for cloud models through the egress broker; double-tap Shift summons the assistant overlay</span></li>
        <li><span class="d">Jul 25</span><span><code>dev.lisaos.Context1</code>: scoped, ledgered retrieval and per-app memory</span></li>
        <li><span class="d">Jul 25</span><span>A/B-update emergency-mode bug class fixed and CI-gated (mounts by partition label, gpt-auto off); the full stack passes e2e natively on ARM64 in containers — bootable Apple Silicon images in progress</span></li>
        <li><span class="d">Jul 24</span><span>Intelligence panel in the forked gnome-control-center: providers, local models, and "Sign in with Claude / ChatGPT" OAuth via the egress broker (ADR-0010/0012/0015)</span></li>
        <li><span class="d">Jul 24</span><span>Lisa Assistant: a persistent GJS chat window — local + cloud models, streaming, ledgered; Super+C opens it (ADR-0015)</span></li>
        <li><span class="d">Jul 24</span><span>Reverse-DNS rename to <code>dev.lisaos.*</code> / <code>app.lisaos.*</code> (ADR-0016) — ships in the next release</span></li>
      </ul>
    </section>

    <section class="sec anchor">
      <h2>Release notes</h2>
      <p>Straight from <a :href="`${repo}/releases`">GitHub Releases</a> — the same channel installed systems update from.</p>
      <template v-if="releases.length">
        <div v-for="r in releases" :key="r.tag" class="rel">
          <div class="rhead">
            <a class="tag" :href="r.url" target="_blank" rel="noopener">{{ r.tag }}</a>
            <span class="d">{{ fmtDate(r.date) }}</span>
            <span v-if="r.prerelease" class="pre">pre-release</span>
            <span v-if="r.name && r.name !== r.tag" style="color:var(--ink-soft);font-size:14px">{{ r.name }}</span>
          </div>
          <pre v-if="r.body" class="relbody"><code>{{ r.body }}</code></pre>
          <div v-else class="empty">No release notes on this one.</div>
        </div>
      </template>
      <p v-else>Could not reach the GitHub API just now — see <a :href="`${repo}/releases`">the releases page on GitHub</a>.</p>
    </section>
  </div>
</template>
