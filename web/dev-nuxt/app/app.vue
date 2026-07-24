<script setup lang="ts">
const colorMode = useColorMode()
const toggle = () => { colorMode.preference = colorMode.value === 'dark' ? 'light' : 'dark' }
const { loggedIn, user, clear } = useUserSession()
const route = useRoute()
const loginNote = computed(() => {
  if (route.query.login === 'unavailable') return 'GitHub sign-in is being set up — check back soon.'
  if (route.query.login === 'error') return 'GitHub sign-in failed — please try again.'
  return ''
})
const { data: issues } = await useFetch('/api/good-first-issues', { default: () => [] })
const repo = 'https://github.com/Lisa-AgenticOS/lisa-os'
const releases = `${repo}/releases/latest`
const wm = '<path d="M20.3932 7C19.7481 7 19.17 6.84919 18.6589 6.54758C18.1478 6.23758 17.7415 5.81867 17.4399 5.29084C17.1466 4.76302 17 4.16816 17 3.50628C17 2.83603 17.1508 2.23698 17.4524 1.70916C17.7624 1.18133 18.1813 0.766607 18.7092 0.464991C19.237 0.154997 19.8318 0 20.4937 0C21.1556 0 21.7463 0.154997 22.2657 0.464991C22.7935 0.766607 23.2083 1.18133 23.5099 1.70916C23.8199 2.23698 23.9791 2.83603 23.9874 3.50628L23.5978 3.8079C23.5978 4.41113 23.4554 4.95571 23.1706 5.44165C22.8941 5.91921 22.5129 6.30042 22.0269 6.58528C21.5494 6.86176 21.0048 7 20.3932 7ZM20.4937 6.12029C20.9797 6.12029 21.4111 6.00718 21.7882 5.78097C22.1735 5.55476 22.4752 5.24476 22.693 4.85099C22.9192 4.44883 23.0323 4.0006 23.0323 3.50628C23.0323 3.00359 22.9192 2.55536 22.693 2.16158C22.4752 1.7678 22.1735 1.45781 21.7882 1.2316C21.4111 0.997008 20.9797 0.879713 20.4937 0.879713C20.0162 0.879713 19.5847 0.997008 19.1993 1.2316C18.8139 1.45781 18.5081 1.7678 18.2819 2.16158C18.0557 2.55536 17.9425 3.00359 17.9425 3.50628C17.9425 4.0006 18.0557 4.44883 18.2819 4.85099C18.5081 5.24476 18.8139 5.55476 19.1993 5.78097C19.5847 6.00718 20.0162 6.12029 20.4937 6.12029ZM23.4973 6.93716C23.3549 6.93716 23.2376 6.89527 23.1454 6.81149C23.0533 6.71933 23.0072 6.60203 23.0072 6.4596V4.31059L23.246 3.31777L23.9874 3.50628V6.4596C23.9874 6.60203 23.9414 6.71933 23.8492 6.81149C23.757 6.89527 23.6397 6.93716 23.4973 6.93716Z"/><path d="M13.0366 7C12.4944 7 11.9569 6.91622 11.4239 6.74865C10.891 6.58109 10.4591 6.32974 10.1283 5.99461C10.0272 5.89408 9.98586 5.78097 10.0042 5.6553C10.0226 5.52962 10.0915 5.42071 10.211 5.32855C10.3396 5.24476 10.4729 5.21125 10.6107 5.22801C10.7485 5.24476 10.8634 5.29922 10.9553 5.39138C11.1758 5.62597 11.4653 5.8061 11.8236 5.93178C12.1912 6.05745 12.5955 6.12029 13.0366 6.12029C13.7166 6.12029 14.2082 6.01556 14.5114 5.8061C14.8146 5.58827 14.9708 5.32855 14.98 5.02693C14.98 4.72531 14.8238 4.47816 14.5114 4.28546C14.199 4.08438 13.6844 3.92938 12.9677 3.82047C12.0396 3.68642 11.3596 3.45183 10.9277 3.1167C10.4958 2.78157 10.2799 2.3836 10.2799 1.9228C10.2799 1.49551 10.404 1.13944 10.6521 0.854578C10.9002 0.569719 11.231 0.356075 11.6445 0.213645C12.058 0.0712149 12.5128 0 13.009 0C13.6247 0 14.153 0.0963495 14.5941 0.289048C15.0444 0.481747 15.4073 0.741472 15.683 1.06822C15.7749 1.17714 15.8116 1.29025 15.7933 1.40754C15.7749 1.52484 15.7014 1.62119 15.5727 1.69659C15.4624 1.75524 15.3338 1.77618 15.1868 1.75943C15.0489 1.73429 14.9295 1.67145 14.8284 1.57092C14.5987 1.32795 14.3322 1.152 14.029 1.04309C13.7257 0.925793 13.3766 0.867146 12.9814 0.867146C12.4761 0.867146 12.0717 0.963495 11.7685 1.15619C11.4653 1.34051 11.3137 1.5751 11.3137 1.85996C11.3137 2.05266 11.3688 2.22023 11.4791 2.36266C11.5985 2.50509 11.7961 2.63076 12.0717 2.73968C12.3566 2.84859 12.7517 2.94075 13.2571 3.01616C13.9463 3.1167 14.4884 3.2675 14.8835 3.46858C15.2879 3.66128 15.5727 3.89168 15.7381 4.15978C15.9127 4.41951 16 4.70437 16 5.01436C16 5.40814 15.8714 5.75583 15.6141 6.05745C15.366 6.35069 15.0168 6.58109 14.5665 6.74865C14.1255 6.91622 13.6155 7 13.0366 7Z"/><path d="M8.50649 7C8.35065 7 8.22511 6.95734 8.12987 6.87203C8.04329 6.77818 8 6.65448 8 6.50091V0.499086C8 0.345521 8.04329 0.226082 8.12987 0.140768C8.22511 0.0469228 8.35065 0 8.50649 0C8.65368 0 8.77056 0.0469228 8.85714 0.140768C8.95238 0.226082 9 0.345521 9 0.499086V6.50091C9 6.65448 8.95238 6.77818 8.85714 6.87203C8.77056 6.95734 8.65368 7 8.50649 7Z"/><path d="M3.14827 7C2.69853 7 2.58746 6.9825 2.16771 6.9825C1.74196 6.9825 1.36718 6.83375 1.04338 6.65875C0.71957 6.47792 0.464722 6.23 0.278833 5.915C0.0929441 5.59417 0 5.22958 0 4.82125V0.455C0 0.320833 0.0449729 0.212917 0.134919 0.13125C0.224865 0.04375 0.338797 0 0.476715 0C0.608636 0 0.71957 0.04375 0.809516 0.13125C0.893466 0.212917 0.935441 0.320833 0.935441 0.455V4.82125C0.935441 5.07208 1.00659 5.28576 1.11453 5.47826C1.21646 5.67076 1.3588 5.80417 1.54469 5.915C1.73058 6.02 1.92785 6.09875 2.16771 6.09875H2.93232C3.41719 6.09875 3.83178 6.0725 3.95778 6.0725C4.08377 6.0725 6.27221 6.09875 6.53005 6.09875C6.66797 6.09875 6.7789 6.11625 6.86285 6.20375C6.9528 6.28542 7 6.40931 7 6.54348C7 6.67181 6.9528 6.76375 6.86285 6.85125C6.7789 6.93875 6.66797 6.9825 6.53005 6.9825C6.1103 6.9825 4.94716 7 4.49746 7H3.14827Z"/>'
const ghMark = 'M12 .5C5.37.5 0 5.78 0 12.29c0 5.2 3.44 9.61 8.2 11.17.6.11.82-.25.82-.56v-2c-3.34.7-4.04-1.6-4.04-1.6-.55-1.36-1.33-1.72-1.33-1.72-1.09-.73.08-.72.08-.72 1.2.08 1.84 1.22 1.84 1.22 1.07 1.8 2.8 1.28 3.49.98.11-.76.42-1.28.76-1.57-2.67-.3-5.47-1.3-5.47-5.79 0-1.28.47-2.32 1.24-3.14-.13-.3-.54-1.52.11-3.18 0 0 1.01-.32 3.3 1.2a11.6 11.6 0 0 1 6 0c2.29-1.52 3.3-1.2 3.3-1.2.65 1.66.24 2.88.12 3.18.77.82 1.24 1.86 1.24 3.14 0 4.5-2.81 5.48-5.49 5.77.43.36.81 1.08.81 2.18v3.23c0 .31.22.68.83.56A12.02 12.02 0 0 0 24 12.29C24 5.78 18.63.5 12 .5z'
</script>

<template>
  <div class="wrap">
    <nav class="nav">
      <span><svg class="brand" viewBox="0 0 24 7" aria-label="Lisa" style="display:inline-block;vertical-align:middle"><g class="wm" v-html="wm" /></svg><span class="badge">DEV</span></span>
      <div class="menu">
        <a href="#contribute">Contribute</a>
        <a href="#install">Install</a>
        <a href="#sdk">SDK</a>
        <a :href="`${repo}/blob/main/docs/PLAN.md`">Architecture</a>
      </div>
      <div class="right">
        <ClientOnly>
          <button class="tt" type="button" aria-label="Toggle dark mode" title="Light / dark" @click="toggle">
            {{ colorMode.value === 'dark' ? '☀' : '☾' }}
          </button>
        </ClientOnly>
        <div v-if="loggedIn" class="who">
          <img :src="user?.avatar" :alt="user?.login" >
          <span>{{ user?.login }}</span>
          <button type="button" @click="clear()">Sign out</button>
        </div>
        <a v-else class="gh" href="/auth/github">
          <svg viewBox="0 0 24 24"><path :d="ghMark" /></svg>
          Sign in with GitHub
        </a>
      </div>
    </nav>

    <header class="hero">
      <span class="eyebrow">Build &amp; contribute</span>
      <h1 v-if="loggedIn">Welcome back, {{ user?.name }}.</h1>
      <h1 v-else>Lisa OS, for the people building on it.</h1>
      <p>An AI-native, self-updating Linux distribution: local models as a system service, per-app durable context, an append-only Ledger, and an OpenAI-compatible endpoint right on the machine. Flash it, boot it, build against it.</p>
      <div class="row">
        <a class="btn solid" :href="releases">Get the latest image</a>
        <a class="btn line" :href="`${repo}/blob/main/docs/PLAN.md`">Read the plan</a>
      </div>
    </header>

    <section id="contribute" class="sec anchor">
      <h2>Start contributing.</h2>
      <p>Good first issues, straight from the repo. Pick one, comment to claim it, and open a PR.</p>
      <div v-if="issues && issues.length" class="board">
        <a v-for="i in issues" :key="i.number" class="issue" :href="i.url" target="_blank" rel="noopener">
          <span class="n">#{{ i.number }}</span>
          <span class="ti">{{ i.title }}</span>
          <span v-for="l in i.labels.slice(0, 2)" :key="l" class="lb">{{ l }}</span>
        </a>
      </div>
      <div v-else class="board"><div class="empty">No open “good first issue” tickets right now — check <a :href="`${repo}/issues`">all issues</a> or open one.</div></div>
      <div v-if="!loggedIn" class="signin-cta">
        <p v-if="loginNote"><strong>{{ loginNote }}</strong></p>
        <p v-else><strong>Sign in with GitHub</strong> to claim issues and track your open PRs here.</p>
        <a class="gh" href="/auth/github"><svg viewBox="0 0 24 24"><path :d="ghMark" /></svg> Sign in with GitHub</a>
      </div>
    </section>

    <section id="install" class="sec anchor">
      <h2>Install</h2>
      <p>Flash the latest USB image and boot it — it runs from the stick, then installs to disk when you're ready. Updates are A/B with automatic rollback on a bad boot.</p>
      <pre><code><span class="c"># 1. Download lisa-usb-&lt;version&gt;.raw.zst from Releases, then:</span>
zstd -d lisa-usb-*.raw.zst -o lisa.raw
sudo dd if=lisa.raw of=/dev/&lt;your-usb&gt; bs=4M status=progress oflag=sync

<span class="c"># 2. Boot it. To install onto the internal disk (erases it):</span>
lisa install /dev/&lt;internal-disk&gt;

<span class="c"># 3. Update in place later (A/B, auto-rollback on a bad boot):</span>
lisa update --reboot</code></pre>
    </section>

    <section id="sdk" class="sec anchor">
      <h2>SDK</h2>
      <p>Intelligence is an OpenAI-compatible endpoint on the machine — build against it with zero Lisa-specific dependencies. Guided generation means typed output that always parses.</p>
      <pre><code><span class="k">from</span> openai <span class="k">import</span> OpenAI
client = OpenAI(base_url=<span class="k">"http://127.0.0.1:7777/v1"</span>, api_key=<span class="k">"local"</span>)

r = client.chat.completions.create(
    model=<span class="k">"lisa"</span>,
    messages=[{<span class="k">"role"</span>: <span class="k">"user"</span>, <span class="k">"content"</span>: <span class="k">"Extract the recipe.\n\n"</span> + text}],
    response_format={<span class="k">"type"</span>: <span class="k">"json_schema"</span>,
                     <span class="k">"json_schema"</span>: {<span class="k">"name"</span>: <span class="k">"recipe"</span>, <span class="k">"schema"</span>: SCHEMA}})
<span class="c"># always valid JSON for SCHEMA</span></code></pre>
      <p class="after"><a :href="`${repo}/tree/main/docs/sdk/samples`">→ Sample apps</a> · <a :href="`${repo}/tree/main/libs/lisa_flutter`">Dart/Flutter SDK</a></p>
    </section>

    <section id="learn" class="sec anchor">
      <h2>Learn</h2>
      <p>The design is the source of truth, and every non-obvious decision is written down.</p>
      <div class="grid">
        <a class="card" :href="`${repo}/blob/main/docs/PLAN.md`"><h3>Architecture</h3><p>The plan &amp; the whole system design.</p></a>
        <a class="card" :href="`${repo}/blob/main/docs/ROADMAP.md`"><h3>Roadmap</h3><p>What's built, what's next, milestone by milestone.</p></a>
        <a class="card" :href="`${repo}/tree/main/docs/adr`"><h3>Decisions (ADRs)</h3><p>Why the OS is built the way it is.</p></a>
        <a class="card" :href="`${repo}`"><h3>Source</h3><p>The monorepo — OS, daemons, shell, apps.</p></a>
      </div>
    </section>

    <footer><span class="mono">LISA OS · GPL-2.0</span> · <a href="https://lisa-app.common.al">lisaos.app</a> · <a :href="repo">GitHub</a></footer>
  </div>
</template>
