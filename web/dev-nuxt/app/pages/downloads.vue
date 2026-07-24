<script setup lang="ts">
// Live releases from the GitHub API via /api/releases.
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
const { data: releases } = await useFetch('/api/releases', { default: () => [] })
const latest = computed(() => releases.value.find((r: any) => !r.prerelease) || releases.value[0])
const fmtSize = (n: number) => {
  if (!n && n !== 0) return ''
  if (n >= 1 << 30) return (n / (1 << 30)).toFixed(2) + ' GB'
  if (n >= 1 << 20) return (n / (1 << 20)).toFixed(1) + ' MB'
  return Math.max(1, Math.round(n / 1024)) + ' KB'
}
const fmtDate = (d: string) => (d ? d.slice(0, 10) : '')
useHead({ title: 'Downloads — Lisa OS developers' })
</script>

<template>
  <div>
    <header class="pagehead">
      <span class="eyebrow">Downloads</span>
      <h1>Releases.</h1>
      <p class="lede">GitHub Releases <em>are</em> the update channel: each release ships the dd-able USB image for humans plus the sysupdate transfer set and <code>SHA256SUMS</code> for machines. Installed systems pull these automatically; below is the same feed, live from the GitHub API.</p>
    </header>

    <section class="sec anchor">
      <h2>Latest<template v-if="latest"> — {{ latest.tag }}</template></h2>
      <p>Flash the USB image, boot from the stick, and install to disk when ready (see <NuxtLink to="/docs/getting-started">Getting started</NuxtLink>):</p>
      <pre><code><span class="c"># 1. Download lisa-usb-&lt;version&gt;.raw.zst below, then:</span>
zstd -d lisa-usb-*.raw.zst -o lisa.raw
sudo dd if=lisa.raw of=/dev/&lt;your-usb&gt; bs=4M status=progress oflag=sync

<span class="c"># 2. Boot it. To install onto the internal disk (erases it):</span>
lisa install /dev/&lt;internal-disk&gt;

<span class="c"># 3. Update in place later (A/B, auto-rollback on a bad boot):</span>
lisa update --reboot</code></pre>
      <p v-if="!releases.length">Could not reach the GitHub API just now — see <a :href="`${repo}/releases`">the releases page on GitHub</a>.</p>
    </section>

    <section v-if="releases.length" class="sec anchor">
      <h2>All releases</h2>
      <p>The ten most recent. Verify every download against the release's <code>SHA256SUMS</code>.</p>
      <div v-for="r in releases" :key="r.tag" class="rel">
        <div class="rhead">
          <a class="tag" :href="r.url" target="_blank" rel="noopener">{{ r.tag }}</a>
          <span class="d">{{ fmtDate(r.date) }}</span>
          <span v-if="r.prerelease" class="pre">pre-release</span>
          <span v-if="r.name && r.name !== r.tag" style="color:var(--ink-soft);font-size:14px">{{ r.name }}</span>
        </div>
        <a v-for="a in r.assets" :key="a.name" class="asset" :href="a.url">
          <span class="an">{{ a.name }}</span>
          <span class="sz">{{ fmtSize(a.size) }}</span>
        </a>
        <div v-if="!r.assets.length" class="empty">No downloadable assets on this release.</div>
      </div>
    </section>
  </div>
</template>
