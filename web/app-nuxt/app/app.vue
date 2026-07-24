<script setup lang="ts">
const colorMode = useColorMode()
const toggle = () => { colorMode.preference = colorMode.value === 'dark' ? 'light' : 'dark' }
const releases = 'https://github.com/Lisa-AgenticOS/lisa-os/releases/latest'
const repo = 'https://github.com/Lisa-AgenticOS/lisa-os'
const docs = 'https://lisa-dev.common.al'
const wm = '<path d="M20.3932 7C19.7481 7 19.17 6.84919 18.6589 6.54758C18.1478 6.23758 17.7415 5.81867 17.4399 5.29084C17.1466 4.76302 17 4.16816 17 3.50628C17 2.83603 17.1508 2.23698 17.4524 1.70916C17.7624 1.18133 18.1813 0.766607 18.7092 0.464991C19.237 0.154997 19.8318 0 20.4937 0C21.1556 0 21.7463 0.154997 22.2657 0.464991C22.7935 0.766607 23.2083 1.18133 23.5099 1.70916C23.8199 2.23698 23.9791 2.83603 23.9874 3.50628L23.5978 3.8079C23.5978 4.41113 23.4554 4.95571 23.1706 5.44165C22.8941 5.91921 22.5129 6.30042 22.0269 6.58528C21.5494 6.86176 21.0048 7 20.3932 7ZM20.4937 6.12029C20.9797 6.12029 21.4111 6.00718 21.7882 5.78097C22.1735 5.55476 22.4752 5.24476 22.693 4.85099C22.9192 4.44883 23.0323 4.0006 23.0323 3.50628C23.0323 3.00359 22.9192 2.55536 22.693 2.16158C22.4752 1.7678 22.1735 1.45781 21.7882 1.2316C21.4111 0.997008 20.9797 0.879713 20.4937 0.879713C20.0162 0.879713 19.5847 0.997008 19.1993 1.2316C18.8139 1.45781 18.5081 1.7678 18.2819 2.16158C18.0557 2.55536 17.9425 3.00359 17.9425 3.50628C17.9425 4.0006 18.0557 4.44883 18.2819 4.85099C18.5081 5.24476 18.8139 5.55476 19.1993 5.78097C19.5847 6.00718 20.0162 6.12029 20.4937 6.12029ZM23.4973 6.93716C23.3549 6.93716 23.2376 6.89527 23.1454 6.81149C23.0533 6.71933 23.0072 6.60203 23.0072 6.4596V4.31059L23.246 3.31777L23.9874 3.50628V6.4596C23.9874 6.60203 23.9414 6.71933 23.8492 6.81149C23.757 6.89527 23.6397 6.93716 23.4973 6.93716Z"/><path d="M13.0366 7C12.4944 7 11.9569 6.91622 11.4239 6.74865C10.891 6.58109 10.4591 6.32974 10.1283 5.99461C10.0272 5.89408 9.98586 5.78097 10.0042 5.6553C10.0226 5.52962 10.0915 5.42071 10.211 5.32855C10.3396 5.24476 10.4729 5.21125 10.6107 5.22801C10.7485 5.24476 10.8634 5.29922 10.9553 5.39138C11.1758 5.62597 11.4653 5.8061 11.8236 5.93178C12.1912 6.05745 12.5955 6.12029 13.0366 6.12029C13.7166 6.12029 14.2082 6.01556 14.5114 5.8061C14.8146 5.58827 14.9708 5.32855 14.98 5.02693C14.98 4.72531 14.8238 4.47816 14.5114 4.28546C14.199 4.08438 13.6844 3.92938 12.9677 3.82047C12.0396 3.68642 11.3596 3.45183 10.9277 3.1167C10.4958 2.78157 10.2799 2.3836 10.2799 1.9228C10.2799 1.49551 10.404 1.13944 10.6521 0.854578C10.9002 0.569719 11.231 0.356075 11.6445 0.213645C12.058 0.0712149 12.5128 0 13.009 0C13.6247 0 14.153 0.0963495 14.5941 0.289048C15.0444 0.481747 15.4073 0.741472 15.683 1.06822C15.7749 1.17714 15.8116 1.29025 15.7933 1.40754C15.7749 1.52484 15.7014 1.62119 15.5727 1.69659C15.4624 1.75524 15.3338 1.77618 15.1868 1.75943C15.0489 1.73429 14.9295 1.67145 14.8284 1.57092C14.5987 1.32795 14.3322 1.152 14.029 1.04309C13.7257 0.925793 13.3766 0.867146 12.9814 0.867146C12.4761 0.867146 12.0717 0.963495 11.7685 1.15619C11.4653 1.34051 11.3137 1.5751 11.3137 1.85996C11.3137 2.05266 11.3688 2.22023 11.4791 2.36266C11.5985 2.50509 11.7961 2.63076 12.0717 2.73968C12.3566 2.84859 12.7517 2.94075 13.2571 3.01616C13.9463 3.1167 14.4884 3.2675 14.8835 3.46858C15.2879 3.66128 15.5727 3.89168 15.7381 4.15978C15.9127 4.41951 16 4.70437 16 5.01436C16 5.40814 15.8714 5.75583 15.6141 6.05745C15.366 6.35069 15.0168 6.58109 14.5665 6.74865C14.1255 6.91622 13.6155 7 13.0366 7Z"/><path d="M8.50649 7C8.35065 7 8.22511 6.95734 8.12987 6.87203C8.04329 6.77818 8 6.65448 8 6.50091V0.499086C8 0.345521 8.04329 0.226082 8.12987 0.140768C8.22511 0.0469228 8.35065 0 8.50649 0C8.65368 0 8.77056 0.0469228 8.85714 0.140768C8.95238 0.226082 9 0.345521 9 0.499086V6.50091C9 6.65448 8.95238 6.77818 8.85714 6.87203C8.77056 6.95734 8.65368 7 8.50649 7Z"/><path d="M3.14827 7C2.69853 7 2.58746 6.9825 2.16771 6.9825C1.74196 6.9825 1.36718 6.83375 1.04338 6.65875C0.71957 6.47792 0.464722 6.23 0.278833 5.915C0.0929441 5.59417 0 5.22958 0 4.82125V0.455C0 0.320833 0.0449729 0.212917 0.134919 0.13125C0.224865 0.04375 0.338797 0 0.476715 0C0.608636 0 0.71957 0.04375 0.809516 0.13125C0.893466 0.212917 0.935441 0.320833 0.935441 0.455V4.82125C0.935441 5.07208 1.00659 5.28576 1.11453 5.47826C1.21646 5.67076 1.3588 5.80417 1.54469 5.915C1.73058 6.02 1.92785 6.09875 2.16771 6.09875H2.93232C3.41719 6.09875 3.83178 6.0725 3.95778 6.0725C4.08377 6.0725 6.27221 6.09875 6.53005 6.09875C6.66797 6.09875 6.7789 6.11625 6.86285 6.20375C6.9528 6.28542 7 6.40931 7 6.54348C7 6.67181 6.9528 6.76375 6.86285 6.85125C6.7789 6.93875 6.66797 6.9825 6.53005 6.9825C6.1103 6.9825 4.94716 7 4.49746 7H3.14827Z"/>'
</script>

<template>
  <div class="wrap">
    <nav class="nav">
      <svg class="brand" viewBox="0 0 24 7" aria-label="Lisa"><g class="wm" v-html="wm" /></svg>
      <div class="menu">
        <a href="#why">Why Lisa</a>
        <a href="#ledger">The Ledger</a>
        <a href="#status">Status</a>
        <a :href="docs">Docs</a>
      </div>
      <div class="right">
        <a class="ghost" :href="repo">GitHub</a>
        <ClientOnly>
          <button class="tt" type="button" aria-label="Toggle dark mode" title="Light / dark" @click="toggle">
            {{ colorMode.value === 'dark' ? '☀' : '☾' }}
          </button>
        </ClientOnly>
        <a class="btn solid" :href="releases">Download</a>
      </div>
    </nav>

    <header class="hero">
      <span class="eyebrow">AI-native Linux · runs on your hardware</span>
      <h1>A computer that shows its work.</h1>
      <p class="lede">Lisa OS builds intelligence into the system, running <b>locally, by default</b>. Nothing leaves your machine without your say-so — and every model call, every file it reads, every action it takes is written to a record you can read.</p>
      <div class="row">
        <a class="btn solid lg" :href="releases">Download Lisa OS</a>
        <a class="btn line lg" :href="docs">Read the docs</a>
      </div>
    </header>

    <div id="ledger" class="stmt" role="img" aria-label="A Lisa Ledger statement: local calls stay on the machine; one remote call is flagged as leaving.">
      <div class="head">
        <span class="l">Ledger · session statement</span>
        <span class="r">Thu 09:41 · 128 events today</span>
      </div>
      <div class="wide"><table>
        <tbody>
          <tr><td class="t">09:41:22</td><td class="kind">inference.generate</td><td>qwen3-0.6b</td><td class="local">on this machine</td><td class="num">420 tok</td></tr>
          <tr><td class="t">09:41:22</td><td class="kind">context.search</td><td>~/notes</td><td class="local">on this machine</td><td class="num">3 hits</td></tr>
          <tr class="egress"><td class="t">09:44:07</td><td class="kind">remote.generate</td><td>claude ↗</td><td class="leaves">LEAVES · consented</td><td class="num">1,240 tok</td></tr>
          <tr><td class="t">09:44:09</td><td class="kind">remote.complete</td><td>claude</td><td class="local">ok</td><td class="num">—</td></tr>
          <tr><td class="t">09:52:10</td><td class="kind">inference.embed</td><td>qwen3-0.6b</td><td class="local">on this machine</td><td class="num">1,024-dim</td></tr>
        </tbody>
      </table></div>
      <div class="foot">Append-only · enforced by the system · the <span class="amber">amber row</span> is the only thing that left your hardware, and only because you allowed it.</div>
    </div>
    <p class="caption">This is real: the Ledger gates every model call before it runs. No entry, no action.</p>

    <section id="why" class="sec">
      <div class="lead-2">
        <span class="eyebrow">Why Lisa</span>
        <h2>Private because of how it's built — not because of a promise.</h2>
        <p>The warmth of a personal assistant that sees, hears, and helps, living on your hardware instead of someone else's data center.</p>
      </div>
      <div class="rows">
        <div class="frow"><span class="mk">◆</span><div><h3>Local by mechanism</h3><p>Models run on your machine. The daemons that read your context have no network access at all — privacy enforced by the system, not by policy. Turning it off isn't a setting; there's nothing to turn off.</p></div></div>
        <div class="frow"><span class="mk amber">↗</span><div><h3>Private by default</h3><p>Nothing offloads to a cloud provider unless you switch it on, per scope. When something would leave your hardware it's marked in a distinct amber, in the interface and in the record — you always know.</p></div></div>
        <div class="frow"><span class="mk">≡</span><div><h3>Yours, on the record</h3><p>Every model call, every piece of context read, every action taken lands in an append-only Ledger you can read in plain English. No black box, no telemetry you can't see.</p></div></div>
      </div>
    </section>

    <section class="sec">
      <div class="lead-2">
        <span class="eyebrow">The assistant</span>
        <h2>Talk to a local model — or your own cloud, on the record.</h2>
        <p>Chat with the model that runs on your machine, or sign in to a provider you already pay for. Either way, the turn is streamed, and the egress is marked.</p>
      </div>
      <div class="win" role="img" aria-label="The Lisa Assistant chat window">
        <div class="bar"><i /><i /><i /><span class="t">Lisa Assistant</span>
          <span class="m">claude · <span class="a">leaves this machine</span></span></div>
        <div class="chat">
          <div class="bub you"><span class="w">YOU</span>Summarize the three files I dropped in ~/notes today.</div>
          <div class="bub lisa"><span class="w">LISA</span>Across all three: the roadmap moved M6 up, the meeting settled the ledger schema, and the spec adds an offline mode.</div>
        </div>
      </div>
    </section>

    <section id="status" class="sec">
      <div class="lead-2">
        <span class="eyebrow">Alpha, in the open</span>
        <h2>What works today.</h2>
        <p>We build in the open and we're honest about the state. Real, on real hardware, now.</p>
      </div>
      <div class="status">
        <div class="col">
          <h4>Working now</h4>
          <ul>
            <li><span class="c" />Bootable, self-updating immutable OS — A/B slots, auto-rollback</li>
            <li><span class="c" />Local inference + guided generation as a system service</li>
            <li><span class="c" />Append-only audit Ledger, enforced by the system</li>
            <li><span class="c" />On-device context search with per-app permissions</li>
            <li><span class="c" />Bring-your-own cloud providers — signed in, off by default</li>
            <li><span class="c" />A chat assistant: local and cloud models, streaming, ledgered</li>
          </ul>
        </div>
        <div class="col soon">
          <h4>On the way</h4>
          <ul>
            <li><span class="c" />Native AI panel in Settings for models &amp; providers</li>
            <li><span class="c" />Always-on ambient assistant</li>
            <li><span class="c" />Apps you can generate and install locally</li>
            <li><span class="c" />Voice + wake-word loop, fully on device</li>
          </ul>
        </div>
      </div>
    </section>

    <div class="close">
      <span class="eyebrow">Take your computing home</span>
      <h2>Flash it, boot it, read the record.</h2>
      <p>Try the alpha on real hardware. It's free and open source — your machine, your models, your record.</p>
      <div class="row">
        <a class="btn solid lg" :href="releases">Download Lisa OS</a>
        <a class="btn line lg" :href="repo">View the source</a>
      </div>
    </div>

    <footer>
      <span class="mono">LISA OS · GPL-2.0 · BUILT IN THE OPEN</span>
      <span><a :href="repo">GitHub</a> · <a :href="docs">Docs</a> · lisaos.dev</span>
    </footer>
  </div>
</template>
