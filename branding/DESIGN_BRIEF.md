# Lisa OS — design brief / prompt

Paste the block below into Claude (with Artifacts on) or hand it to a
designer to start on Lisa OS interfaces. It carries the full brand system.
Keep it in sync with `branding/` and `docs/VISION.md`.

---

## PROMPT (copy from here)

You are designing the interface language for **Lisa OS** — an AI-native
Linux desktop where intelligence is a built-in system service, everything
runs **locally by default**, nothing leaves the machine without explicit
consent, and every AI action is recorded in a plain-English audit log.
The feeling to aim for: **"Her, but yours"** — the warmth and intimacy of
a personal assistant, without the cloud, without leaving home. A computer
that's genuinely *yours* to think with. Calm, warm, human — never cold,
corporate, or sci-fi.

Design a cohesive **design system + key screens**. Deliver theme-aware
(light **and** dark) high-fidelity mockups as self-contained HTML/CSS
artifacts, plus a documented token sheet. Favor restraint over decoration
(elementary-OS / GNOME-native sensibility): generous whitespace, one clear
action per view, quiet color, humane copy.

### Brand foundations

**Logo — wordmark-first.** The identity is the **"Lisa" logotype** itself
(a clean geometric-humanist wordmark), not an icon. It ships in violet
(`lisa-wordmark.svg`) and reversed white (`lisa-wordmark-white.svg`) —
use white whenever it sits on violet/dark. For icon-only slots (favicon,
os-release logo, app grid) there's an **L monogram** tile
(`lisa-mark.svg`: white L on a violet squircle). No orb.

**Color tokens** (starting system — refine, but keep the warmth and the
reserved amber):

Brand — violet (two steps, one family). Deep for identity, livelier for UI.
- Violet 700 `#4F378B`  — BRAND / the wordmark / deep surfaces
- Violet 500 `#6D45C9`  — PRIMARY UI: buttons, toggles, links, the L tile
- Violet 300 `#9B7BE8`  — hovers, gradient top, soft accents
- The wordmark reverses to WHITE on violet/dark. Do NOT use orange as a UI
  accent — it must not compete with the reserved amber.

Warm neutrals (the UI is built from these — warm-tinted, never cold gray)
- Ink 900 `#2B2320`    — primary text on light
- Ink 700 `#4A423D`
- Ink 500 `#6E635C`    — secondary text
- Ink 300 `#9A8F88`    — tertiary / captions
- Line 200 `#E7DED8`   — borders, dividers
- Warm White `#FFF1E9` — on-dark text, warm surfaces
- Paper `#FAF7F5`      — light app background
- Surface `#FFFFFF`    — light cards

Dark surfaces
- Base `#1B1917` · Elevated `#262220` · Card `#302B28` · Line `#3C3531`
  · text primary `#FFF1E9`, secondary `#B7ABA3`

Semantic (warm-leaning, use sparingly)
- Success `#3E9B6B` · Warning `#E0A030` · Error `#D6453C` · Info `#4A7DB0`

**RESERVED — do not use for brand or generic UI:**
- Egress amber `#E66100` means exactly one thing: **"this leaves your
  hardware."** It marks any remote/offload action and its consent toggles.
  It must never read as a brand color — the violet brand already sits far
  from it, so the warning always reads as a warning. Keep it that way.

**Typography — Rubik is the OS font.** A warm, slightly-rounded geometric
sans (OFL, shipped in the image, set as the GNOME UI font). One family
does the work: UI, body, and headings (heavier weights + tighter tracking
at display sizes). Mono (Ledger, code, model ids): the system monospace.
The "Lisa" wordmark keeps its own geometric letterforms — Rubik sits
comfortably beside it. All OFL/open.

**Motion.** Gentle, brief, purposeful — a soft "listening" pulse for the
assistant, calm cross-fades, no bounce/flash. Nothing that reads as
surveillance or hype.

**Iconography.** GNOME symbolic style (monoline, 16px grid) for UI;
the violet mark only for the OS/brand.

### Principles

1. **Local by default, legible always.** Anything that could leave the
   machine is amber and consented; the default state is "Nothing leaves
   this machine." Provenance (where a piece of context came from) is
   visible, not hidden.
2. **One voice.** The assistant, the shell, and first-party apps feel like
   one designed thing (tokens-first — every surface reads the same theme).
3. **Humane, not clever.** Plain words, honest states ("alpha — here's
   what works"), no dark patterns, no fake urgency.
4. **Accessible.** WCAG AA contrast (body text on Ink, not on the brand);
   full keyboard paths; light + dark parity.

### Screens to design (priority order)

1. **Assistant overlay** — a Spotlight-style summon (a warm, calm chat
   surface over the desktop): idle prompt, a streaming answer, an
   attached-context chip with provenance, and the amber "this would use a
   provider" moment. The signature surface.
2. **Ambient / voice** — the always-on-but-quiet states: idle, wake
   ("Hey Lisa"), listening pulse, answering, and a visible **hard mute**.
   Must feel companionable, never like a hot mic.
3. **Settings › Intelligence** — the AI panel (native GNOME/libadwaita):
   **Local models** (what runs on this machine, fit badges, one-click
   Get) and **Providers & privacy** (bring-your-own accounts, keys, and
   the amber per-scope "may leave this machine" switches).
4. **Semantic launcher** — type-to-find apps/actions/answers; the calc
   and unit lanes; a result that's an assistant answer.
5. **Ledger app** — the plain-English audit log: every model call,
   context grant, tool run — filterable, honest, reassuring not scary.
6. **Consent / permission chips** — the portal moment an app asks to read
   files/mail/screen; per-scope, revocable, provenance-clear.
7. **First-boot (OOBE)** — warm welcome, language/time, the one honest
   privacy explanation, optional local-model download by hardware tier.
8. **First-party apps** — Notes + Recorder as reference apps that show the
   design system (AI-native widgets: streaming text, consent chip, context
   attachment, voice input, ledger badge).
9. **lisaos.app landing** — the marketing hero (mark + "A computer that's
   genuinely yours to think with"), three pillars (local / private /
   yours), screenshots, one **Get Lisa OS** CTA.

### Platform notes

- Desktop surfaces target **GTK4 + libadwaita** visuals (GNOME 50); match
  its layout grammar (header bars, preference groups, rows, toolbars).
- `lisa_ui` is the reserved name for the **shared GJS/GTK4 library** for
  first-party apps — not yet written (ADR-0047 §6)
  (ADR-0047 §6), reading the same tokens. The Flutter package that held
  the name is parked, unshipped, and not the default.
- Everything ships **light and dark**; the theme is one token source read
  by GTK, the shell, and the apps alike.

Deliver: (a) a token/color/type sheet, (b) the assistant overlay + ambient
+ Intelligence panel as polished light/dark mockups first, then the rest.
Explain the choices briefly. Keep it warm, quiet, and unmistakably Lisa.

## (end prompt)
