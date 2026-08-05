<script setup lang="ts">
// Design guidelines — values from app/assets/css/main.css (this site),
// branding/tokens.json, docs/notes/design-direction.md, ADR-0038 (tokens),
// ADR-0047 (GJS + GTK4/Adwaita is the one toolkit; the Flutter kit is parked).
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'Design — Lisa OS developers' })

// The palette is READ from the generated tokens, never retyped. This
// page used to hardcode swatch hexes — and by 2026-08-05 they were the
// site's old hand-picked values, so the page documenting the design
// system had drifted off the design system. Caught by check-tokens.py
// the day `web` joined its SURFACES list. Derived, it cannot drift.
import { TOKENS } from '~/assets/tokens.js'

// The brand ramp, in tokens.json order.
const ramp: [string, string][] = (
  ['violet-700', 'violet-500', 'violet-300'] as const
).map(k => [k.replace('violet-', ''), TOKENS[k]] as [string, string])

// Role, light value, dark value, meaning. Light and dark both come from
// tokens.json — the site's semantic aliases map onto these names (see
// app/assets/css/main.css).
const tokens: [string, string, string, string][] = [
  ['--paper', TOKENS['paper'], TOKENS['base'], 'The page.'],
  ['--ground', TOKENS['surface'], TOKENS['elevated'], 'Raised surfaces: boards, cards, tables.'],
  ['--ink', TOKENS['ink-900'], TOKENS['text-primary'], 'Primary text.'],
  ['--ink-soft', TOKENS['ink-500'], TOKENS['text-secondary'], 'Secondary text — ledes, descriptions.'],
  ['--ink-faint', TOKENS['ink-300'], TOKENS['text-secondary'], 'Tertiary: dates, labels, footnotes.'],
  ['--line', TOKENS['line-200'], TOKENS['line'], 'Hairline rules and borders.'],
  ['--violet', TOKENS['violet-500'], TOKENS['violet-300'], 'The brand accent — actions, emphasis.'],
  ['--violet-ink', TOKENS['violet-700'], TOKENS['violet-300'], 'Violet tuned for text and links.'],
  ['--amber', TOKENS['egress'], TOKENS['warning'], 'Egress — anything that leaves your hardware.'],
  ['--green', TOKENS['success'], TOKENS['success'], 'Verified / healthy state.']
]</script>

<template>
  <div>
    <header class="pagehead">
      <span class="eyebrow">Design</span>
      <h1>One voice, every surface.</h1>
      <p class="lede">Lisa's direction is elementary-inspired: restrained typography, quiet color, humane defaults, one visual voice (recorded in <a :href="`${repo}/blob/main/docs/notes/design-direction.md`">docs/notes/design-direction.md</a>). Tokens first — <a :href="`${repo}/blob/main/branding/tokens.json`">branding/tokens.json</a> is the one source, and <code>branding/generate-tokens.py</code> emits every consumable sheet from it: a CSS file carrying GTK <code>@define-color</code> names and custom properties, an ES module for GJS, and the copies these two websites read. A lint gate rebuilds them and fails on drift.</p>
    </header>

    <section class="sec anchor">
      <h2>The violet ramp</h2>
      <p>Everything brand-colored derives from the Lisa violet, <code>#6D45C9</code> — the same seed the GDM greeter, the wordmark accents, and every generated token sheet use. The three brand steps <code>branding/tokens.json</code> sanctions — every surface, this site included, resolves to these:</p>
      <div class="ramp">
        <div v-for="[step, hex] in ramp" :key="step" class="chip" :style="{ background: hex }">
          <span :style="{ color: step === '300' ? 'var(--ink)' : 'var(--color-warm-white)' }">{{ step }}</span>
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
        <li><strong>Tokens first.</strong> One token source is the design language. The sheets that exist today are CSS (GTK <code>@define-color</code> + custom properties) and an ES module for GJS; a Qt sheet is intent, not code. One voice across every surface.</li>
        <li><strong>Identity where freedom is total.</strong> First-party apps carry the look first — our apps, our Shell fork (ADR-0038). GTK4/libadwaita and Mutter are never forked: toolkit and compositor are foundation, not experience, and where a Lisa app does not exist yet the stock GNOME app ships <em>unpatched</em> (ADR-0048).</li>
        <li><strong>Restraint.</strong> Quiet color, hairline rules, generous whitespace; the violet earns emphasis by being scarce.</li>
        <li><strong>Honesty in chrome.</strong> Egress is amber, always; consent is a plain question; streaming output shows a stop affordance and its provenance.</li>
      </ul>
    </section>
  </div>
</template>
