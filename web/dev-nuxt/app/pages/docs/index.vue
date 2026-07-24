<script setup lang="ts">
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'Docs — Lisa OS developers' })
</script>

<template>
  <div>
    <span class="eyebrow">Documentation</span>
    <h1>Build on Lisa OS.</h1>
    <p class="lede">Lisa makes intelligence a system service: one daemon owns the compute and serves every app, every app gets durable memory and scoped context, every action lands in an append-only Ledger. These docs are distilled from the repo's own specs — <code>docs/PLAN.md</code> is the source of truth.</p>

    <div class="grid" style="margin-top:34px">
      <NuxtLink class="card" to="/docs/getting-started"><h3>Getting started</h3><p>Flash the USB image, boot it, install to disk, stay updated — A/B slots with automatic rollback.</p></NuxtLink>
      <NuxtLink class="card" to="/docs/architecture"><h3>Architecture</h3><p>The daemon map, the portal trust boundary, the Ledger, and the egress rules — plus every ADR.</p></NuxtLink>
      <NuxtLink class="card" to="/docs/apps"><h3>Building apps</h3><p>The GJS shell surfaces, the <code>lisa-app</code> launcher, the app update channel, and the Flutter lane.</p></NuxtLink>
      <NuxtLink class="card" to="/docs/cli"><h3>CLI reference</h3><p>Every <code>lisa</code> verb, generated from the actual command definitions in the repo.</p></NuxtLink>
    </div>

    <h2>Primary sources</h2>
    <p>Everything here links back to the repo. When the docs and the repo disagree, the repo wins.</p>
    <ul>
      <li><a :href="`${repo}/blob/main/docs/PLAN.md`">docs/PLAN.md</a> — the master plan: architecture and scope, section by section.</li>
      <li><a :href="`${repo}/blob/main/docs/STATUS.md`">docs/STATUS.md</a> — the living "where are we" snapshot, updated as things ship.</li>
      <li><a :href="`${repo}/blob/main/docs/ROADMAP.md`">docs/ROADMAP.md</a> — milestones M0–M7.</li>
      <li><a :href="`${repo}/tree/main/docs/adr`">docs/adr/</a> — every architecture decision record.</li>
      <li><a :href="`${repo}/tree/main/docs/sdk`">docs/sdk/</a> — SDK notes and sample apps.</li>
    </ul>
  </div>
</template>
