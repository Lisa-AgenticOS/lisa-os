<script setup lang="ts">
// The full good-first-issues board + how to get set up.
const { loggedIn } = useUserSession()
const route = useRoute()
const loginNote = computed(() => {
  if (route.query.login === 'unavailable') return 'GitHub sign-in is being set up — check back soon.'
  if (route.query.login === 'error') return 'GitHub sign-in failed — please try again.'
  return ''
})
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'Contribute — Lisa OS developers' })
</script>

<template>
  <div>
    <header class="pagehead">
      <span class="eyebrow">Contribute</span>
      <h1>Start contributing.</h1>
      <p class="lede">Good first issues, straight from the repo. Pick one, comment to claim it, and open a PR. The project is GPL-2.0 and the whole OS — daemons, shell, apps, image pipeline — lives in one monorepo.</p>
    </header>

    <section class="sec anchor">
      <h2>Good first issues</h2>
      <IssuesBoard />
      <div v-if="!loggedIn" class="signin-cta">
        <p v-if="loginNote"><strong>{{ loginNote }}</strong></p>
        <p v-else><strong>Sign in with GitHub</strong> to claim issues and track your open PRs here.</p>
        <GhSignIn />
      </div>
    </section>

    <section class="sec anchor">
      <h2>Get set up</h2>
      <pre><code>git clone {{ repo }}.git &amp;&amp; cd lisa-os
git config core.hooksPath .githooks   <span class="c"># pre-push hook runs the lint gate</span>
just build                            <span class="c"># cargo build --workspace</span>
just lint &amp;&amp; just test                <span class="c"># the CI gate — run before every commit</span></code></pre>
      <p>The Rust workspace builds and tests on macOS <em>and</em> Linux; image and systemd/portal work is Linux-only and runs in CI. Read the component's spec section in <code>docs/PLAN.md</code> before touching it — component READMEs mirror their spec.</p>
    </section>

    <section class="sec anchor">
      <h2>Where to look</h2>
      <div class="grid">
        <a class="card" :href="`${repo}/issues`"><h3>All issues</h3><p>The full tracker — bugs, features, and milestone work.</p></a>
        <a class="card" :href="`${repo}/blob/main/CLAUDE.md`"><h3>Working agreements</h3><p>The operating manual: commands, component map, the rules.</p></a>
        <a class="card" :href="`${repo}/blob/main/docs/PLAN.md`"><h3>The plan</h3><p>Architecture and scope — the source of truth.</p></a>
        <a class="card" :href="`${repo}/blob/main/docs/STATUS.md`"><h3>Status</h3><p>The living "where are we" snapshot — read it first to catch up.</p></a>
      </div>
    </section>
  </div>
</template>
