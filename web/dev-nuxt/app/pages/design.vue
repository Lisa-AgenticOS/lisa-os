<script setup lang="ts">
// Design guidelines — values from app/assets/css/main.css (this site),
// branding/tokens.json, docs/notes/design-direction.md, ADR-0038 (tokens),
// ADR-0047 (GJS + GTK4/Adwaita is the one toolkit; the Flutter kit is parked).
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'Design — Lisa OS developers' })

const ramp: [string, string][] = [
  ['50', '#f4f0fb'], ['100', '#e7def7'], ['200', '#d0bdf0'], ['300', '#b396e6'],
  ['400', '#9169da'], ['500', '#6d45c9'], ['600', '#5b36ad'], ['700', '#4a2c8c'],
  ['800', '#3d2672'], ['900', '#33215e'], ['950', '#211239']
]
// token, light, dark, meaning
const tokens: [string, string, string, string][] = [
  ['--paper', '#FBFAF8', '#0E0C12', 'The page. Warm near-white, near-black in dark.'],
  ['--ground', '#FFFFFF', '#151220', 'Raised surfaces: boards, cards, tables.'],
  ['--ink', '#17151C', '#F3F0F8', 'Primary text.'],
  ['--ink-soft', '#6C6675', '#A9A2BB', 'Secondary text — ledes, descriptions.'],
  ['--ink-faint', '#9C97A6', '#736C85', 'Tertiary: dates, labels, footnotes.'],
  ['--line', '#ECE8F1', '#241E31', 'Hairline rules and borders.'],
  ['--violet', '#6D45C9', '#A183EC', 'The brand accent — actions, emphasis.'],
  ['--violet-ink', '#5B36AD', '#B9A0F4', 'Violet tuned for text and links.'],
  ['--amber', '#D75600', '#FF8A3D', 'Egress — anything that leaves your hardware.'],
  ['--green', '#1F9D57', '#3FCB7F', 'Verified / healthy state.']
]
</script>

<template>
  <div>
    <header class="pagehead">
      <span class="eyebrow">Design</span>
      <h1>One voice, every surface.</h1>
      <p class="lede">Lisa's direction is elementary-inspired: restrained typography, quiet color, humane defaults, one visual voice (recorded in <a :href="`${repo}/blob/main/docs/notes/design-direction.md`">docs/notes/design-direction.md</a>). Tokens first — a single token source is the design language; the shell CSS, GTK4/Adwaita, and Qt surfaces all read from it.</p>
    </header>

    <section class="sec anchor">
      <h2>The violet ramp</h2>
      <p>Everything brand-colored derives from the Lisa violet, <code>#6D45C9</code> — the same seed the GDM greeter, the wordmark accents, and every generated token sheet use. The ramp as this site ships it (<code>--color-lisa-*</code>):</p>
      <div class="ramp">
        <div v-for="[step, hex] in ramp" :key="step" class="chip" :style="{ background: hex }">
          <span :style="{ color: Number(step) >= 400 ? '#fff' : '#211239' }">{{ step }}</span>
        </div>
      </div>
      <div class="tbl" style="margin-top:14px"><table>
        <thead><tr><th>Step</th><th>Value</th></tr></thead>
        <tbody>
          <tr v-for="[step, hex] in ramp" :key="step"><td><code>lisa-{{ step }}</code></td><td><code>{{ hex }}</code></td></tr>
        </tbody>
      </table></div>
    </section>

    <section class="sec anchor">
      <h2>Editorial tokens</h2>
      <p>The portal and the marketing site share one editorial system — paper and ink, a violet accent, amber reserved for a single meaning. Light is the default; dark inverts by token, never ad hoc.</p>
      <div class="board" style="margin-top:18px">
        <div v-for="[tk, light, dark, use] in tokens" :key="tk" class="swrow">
          <span class="dot" :style="{ background: light }" />
          <span class="dot" :style="{ background: dark }" />
          <span class="tk">{{ tk }}</span>
          <span class="val">{{ light }} / {{ dark }}</span>
          <span class="use">{{ use }}</span>
        </div>
      </div>
      <div class="note amber"><strong>Amber means egress.</strong> In the OS, every request that leaves your hardware is marked machine-readably (the <code>remote.*</code> Ledger kinds) and rendered in the dedicated egress color — <code>#E66100</code> with CSS class <code>leaves-hardware</code> (ADR-0010). Settings shows an amber "leaves your hardware" badge per cloud provider. This site's amber tokens are the editorial rendering of the same rule: amber is never decoration.</div>
    </section>

    <section class="sec anchor">
      <h2>Type: Rubik</h2>
      <p>Rubik is the UI face — shipped in the OS image as the system font (gschema override). It is deliberately not bundled with any toolkit: the family name resolves against the OS-installed font and falls back to the platform default sans when absent. Monospace surfaces (code, dates, ledger entries) use the platform mono stack with tabular numerals.</p>
    </section>

    <section class="sec anchor">
      <h2>The lisa_ui widget kit — parked</h2>
      <p><strong>This is not how you build a Lisa app.</strong> <code>libs/lisa_ui</code> is the Flutter kit ADR-0014 designed, and ADR-0047 parked it: unshipped, unproven on hardware, not the default. Every shipped surface is GJS + GTK4/Adwaita. The kit is kept, not deleted, and the name is reserved for the shared GJS library ADR-0047 §6 asks for. Recorded here because the token vocabulary below is the same one every surface reads. From <a :href="`${repo}/blob/main/libs/lisa_ui/lib/lisa_ui.dart`">lisa_ui.dart</a>:</p>
      <div class="tbl"><table>
        <thead><tr><th>Export</th><th>What it is</th></tr></thead>
        <tbody>
          <tr><td><code>lisaSeedColor</code></td><td>The violet seed every Lisa theme derives from: <code>Color(0xFF6D45C9)</code>.</td></tr>
          <tr><td><code>lisaTheme(brightness)</code></td><td>Builds the Lisa <code>ThemeData</code>: Material 3, <code>ColorScheme.fromSeed</code> on the violet, Rubik, and tokens mapped into component shapes (card/dialog/input radius).</td></tr>
          <tr><td><code>LisaTokens</code> / <code>LisaTheme</code></td><td>The design-token set (background, surface, text, accent, danger, radius, spacing, fontSize) with inherited-widget access. <code>LisaTokens.fallback</code> mirrors the design-direction note until the system theme file lands — consumers read tokens, never hardcode.</td></tr>
          <tr><td><code>LisaApp</code></td><td>The root widget every Lisa app starts with: a <code>MaterialApp</code> pre-wired to Lisa theming, light + dark, following the OS mode.</td></tr>
          <tr><td><code>LisaScaffold</code></td><td>A <code>Scaffold</code> with Lisa defaults: an <code>AppBar</code> when a title is set, body inset by <code>SafeArea</code>.</td></tr>
          <tr><td><code>LisaCard</code></td><td>A <code>Card</code> padded by the spacing token; corner radius from the radius token.</td></tr>
          <tr><td><code>LisaStreamText</code></td><td>Streaming model output: accumulates token deltas, shows a stop affordance while streaming, reserves the footnote row for provenance chips.</td></tr>
          <tr><td><code>ConsentChip</code></td><td>The consent affordance for a scope request: states the scope plainly, offers allow / deny, never dark-patterns.</td></tr>
        </tbody>
      </table></div>
      <div class="note">Status: <strong>parked</strong> (ADR-0047 §2). No user-facing app has ever been written with it, the OS image ships no copy, and <code>lisa forge</code> targets GJS. To build an app, read <NuxtLink to="/docs/apps">Building apps</NuxtLink> and run <code>lisa dev check</code>.</div>
    </section>

    <section class="sec anchor">
      <h2>Principles</h2>
      <ul>
        <li><strong>Tokens first.</strong> One token source is the design language — shell CSS, GTK4/libadwaita, and Qt all read it (ADR-0038). One voice across every surface.</li>
        <li><strong>Identity where freedom is total.</strong> First-party apps carry the look first — our widgets, our rules; the GNOME base is kept for portal maturity.</li>
        <li><strong>Restraint.</strong> Quiet color, hairline rules, generous whitespace; the violet earns emphasis by being scarce.</li>
        <li><strong>Honesty in chrome.</strong> Egress is amber, always; consent is a plain question; streaming output shows a stop affordance and its provenance.</li>
      </ul>
    </section>
  </div>
</template>
