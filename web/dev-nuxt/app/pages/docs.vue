<script setup lang="ts">
// Docs shell: left sidebar (sticky on desktop, scrollable row on mobile)
// around the /docs/* child pages.
</script>

<template>
  <div class="docslayout">
    <aside class="side">
      <p class="lbl">Documentation</p>
      <NuxtLink to="/docs">Overview</NuxtLink>
      <NuxtLink to="/docs/getting-started">Getting started</NuxtLink>
      <NuxtLink to="/docs/architecture">Architecture</NuxtLink>
      <NuxtLink to="/docs/apps">Building apps</NuxtLink>
      <NuxtLink to="/docs/cli">CLI reference</NuxtLink>
      <p class="lbl" style="margin-top:22px">Reference</p>
      <NuxtLink to="/api">API reference</NuxtLink>
      <NuxtLink to="/design">Design</NuxtLink>
    </aside>
    <div class="doc">
      <NuxtPage />
    </div>
  </div>
</template>
