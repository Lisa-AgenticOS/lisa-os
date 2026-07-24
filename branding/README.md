# Lisa OS — brand kit

The identity for Lisa OS: warm, personal, local. Not cold tech — a
computer that's genuinely yours to think with ("Her, but yours", see
`docs/VISION.md`).

## Assets

- `lisa-mark.svg` — the mark (512×512 squircle). Used as the OS logo:
  shipped as `lisa-logo` under `hicolor/scalable/apps`, referenced by
  `LOGO=lisa-logo` in os-release, so it shows in Settings › About.
- `lisa-wordmark.svg` — horizontal lockup (mark + "Lisa OS").

**Wordmark-first.** The "Lisa" logotype *is* the brand — a clean
geometric-humanist wordmark, no icon needed. It ships in two fills:
`lisa-wordmark.svg` (violet `#4F378B`, for light backgrounds) and
`lisa-wordmark-white.svg` (reversed, for dark/violet backgrounds — use
white whenever the wordmark sits on color). `lisa-mark.svg` is the **L
monogram** (white L on a violet tile) for square icon slots (favicon, app
grid). The os-release logo (Settings > About) uses the **wordmark**.

## Palette (core)

Two violets, one family: a deep brand violet + a livelier UI primary.

| Token | Hex | Use |
|---|---|---|
| Violet 700 | `#4F378B` | BRAND / the wordmark / deep surfaces |
| Violet 500 | `#6D45C9` | PRIMARY UI — buttons, toggles, links, the tile |
| Violet 300 | `#9B7BE8` | hovers, gradient top |
| White | `#FFFFFF` | the wordmark reversed on violet/dark |
| Ink | `#2B2320` | body text, on-light foreground |
| Warm White | `#FFF1E9` | on-dark text, warm surfaces |

`ANSI_COLOR` in os-release is Violet 500 (`1;38;2;109;69;201`) — the
livelier step reads better on a dark console. The full UI token system
(warm neutrals, dark surfaces, semantic colors) and the surfaces to design
live in [`DESIGN_BRIEF.md`](DESIGN_BRIEF.md) — a ready-to-paste prompt for
designing Lisa OS interfaces.

Orange is deliberately **out** of the brand: egress amber (`#E66100`) is
reserved and load-bearing — it means "leaves your hardware" (ADR-0008,
§5.11). A violet brand keeps the warning unmistakable; nothing in the UI
competes with it.

## Where it's wired

- `os/mkosi/mkosi.postinst.chroot` rebrands os-release (`NAME`/
  `PRETTY_NAME` = "Lisa OS", `ID=lisa`, `LOGO=lisa-logo`, violet
  `ANSI_COLOR`, our URLs).
- `os/mkosi/mkosi.extra/usr/share/icons/hicolor/scalable/apps/lisa-logo.svg`
  — the logo icon the About panel resolves.
- `os/mkosi/mkosi.extra/usr/share/plymouth/themes/lisa/` — the **boot
  splash** (Plymouth `two-step` theme): brand violet with the white
  wordmark + a subtle spinner. `watermark.png` is rasterized from
  `lisa-wordmark.svg` and recolored white-on-transparent (Plymouth reads
  PNG, not SVG). See `os/mkosi/README.md` › Boot splash.

This is a first identity pass. Next: GDM/greeter logo and a default
wallpaper in the same language.
