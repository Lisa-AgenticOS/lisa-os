<script setup lang="ts">
const repo = 'https://github.com/Lisa-AgenticOS/lisa-os'
const docs = 'https://lisa-dev.common.al'
const updated = '30 July 2026'
useHead({
  title: 'Terms — Lisa OS',
  meta: [{ name: 'description', content: 'The terms for using Lisa OS: free software, alpha quality, supplied without warranty.' }],
})
</script>

<template>
  <div class="wrap">
    <nav class="nav">
      <NuxtLink to="/" class="brandtext">Lisa OS</NuxtLink>
      <div class="right">
        <a class="ghost" :href="repo">GitHub</a>
        <NuxtLink class="btn line" to="/">Home</NuxtLink>
      </div>
    </nav>

    <article class="legal">
      <span class="eyebrow">Terms</span>
      <h1>The terms for using Lisa OS.</h1>
      <p class="lede">
        Lisa OS is free software you run on your own hardware. There is no
        account, no subscription and no service to sign up for — which makes
        this shorter than most.
      </p>
      <p class="upd">Last updated {{ updated }}</p>

      <h2>The licence governs the software</h2>
      <p>
        Lisa OS is distributed under the <b>GPL-2.0</b>. That licence, not this
        page, is what grants you the right to use, study, modify and
        redistribute it. Where the two ever appear to disagree, the licence
        wins. The full text ships with the source at
        <a :href="repo">the repository</a>.
      </p>

      <h2>Alpha software, and we mean it</h2>
      <p>
        Lisa OS is under active development and is not finished. It writes to
        disks, replaces bootloaders and updates itself between partitions.
        Install it on hardware you can afford to reinstall, and keep backups of
        anything you care about. Do not run it in production.
      </p>

      <h2>No warranty</h2>
      <p>
        The software is provided <b>"as is", without warranty of any kind</b>,
        express or implied, including but not limited to the warranties of
        merchantability, fitness for a particular purpose and non-infringement.
        To the fullest extent permitted by law, the authors and contributors
        are not liable for any claim, damages or other liability arising from
        the software or its use — including lost data. Sections 11 and 12 of
        the GPL-2.0 say this in the operative language.
      </p>

      <h2>What you are responsible for</h2>
      <ul>
        <li>The accounts you connect, and complying with the terms of those providers — Google, Anthropic, OpenAI and any other.</li>
        <li>The content you process, and your right to process it.</li>
        <li>Reviewing what the assistant proposes before you let it act. The system is built to ask; that only helps if you read the question.</li>
        <li>Using it lawfully.</li>
      </ul>

      <h2>Third-party services</h2>
      <p>
        Cloud models and mail providers are not ours. When you sign in to one,
        your relationship is with that provider under their terms, and their
        handling of your data is theirs to describe. Lisa OS marks these calls
        and records them so you can see exactly when it happens — see
        <NuxtLink to="/privacy">Privacy</NuxtLink>.
      </p>

      <h2>Trade marks</h2>
      <p>
        The GPL covers the code, not the name. "Lisa OS" and the logo are ours;
        please do not use them in a way that suggests we endorse or produced
        something we did not. Fair descriptive use — saying your work is based
        on Lisa OS — is fine and welcome.
      </p>

      <h2>Contributions</h2>
      <p>
        Contributions are accepted under the same GPL-2.0 as the rest of the
        project. There is no separate contributor licence agreement.
      </p>

      <h2>Changes</h2>
      <p>
        These terms may change; the date above moves when they do, and the
        history is public in the repository.
      </p>

      <h2>Contact</h2>
      <p>
        <a :href="repo + '/issues'">Open an issue on GitHub</a>.
      </p>
    </article>

    <footer>
      <span class="mono">LISA OS · GPL-2.0 · BUILT IN THE OPEN</span>
      <span>
        <a :href="repo">GitHub</a> · <a :href="docs">Docs</a> ·
        <NuxtLink to="/privacy">Privacy</NuxtLink> ·
        <NuxtLink to="/terms">Terms</NuxtLink>
      </span>
    </footer>
  </div>
</template>

<style scoped>
.legal { max-width: 46rem; margin: 0 auto; padding: 3rem 0 4rem; }
.legal h1 { margin: .4rem 0 1rem; }
.legal h2 { margin: 2.4rem 0 .6rem; font-size: 1.15rem; }
.legal p, .legal li { line-height: 1.7; }
.legal ul { padding-left: 1.1rem; }
.legal li { margin: .45rem 0; }
.upd { opacity: .6; font-size: .9rem; }
.brandtext { font-weight: 600; letter-spacing: .02em; text-decoration: none; }
</style>
