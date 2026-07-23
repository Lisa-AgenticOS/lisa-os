# Lisa OS — brand kit

The identity for Lisa OS: warm, personal, local. Not cold tech — a
computer that's genuinely yours to think with ("Her, but yours", see
`docs/VISION.md`).

## Assets

- `lisa-mark.svg` — the mark (512×512 squircle). Used as the OS logo:
  shipped as `lisa-logo` under `hicolor/scalable/apps`, referenced by
  `LOGO=lisa-logo` in os-release, so it shows in Settings › About.
- `lisa-wordmark.svg` — horizontal lockup (mark + "Lisa OS").

The mark is a warm presence — an orb, gently lit — on a coral squircle:
a companion that lives on your hardware, not in a cloud. The wordmark's
text uses a system sans fallback; outline it to paths before any print /
external use so it renders identically everywhere.

## Palette

| Token | Hex | Use |
|---|---|---|
| Lisa Coral | `#EE5B3B` | primary brand, accents, the "OS" in the wordmark |
| Coral Light | `#FF9E7A` | gradient top, hovers |
| Ink | `#2B2320` | wordmark text, on-light foreground |
| Warm White | `#FFF1E9` | the orb, on-dark surfaces |

`ANSI_COLOR` in os-release is Lisa Coral (`1;38;2;238;91;59`).

**Not** the egress amber (`#E66100`) — that color is reserved, load-bearing:
it means "leaves your hardware" (ADR-0008, §5.11). Keep the brand coral
distinct from it so the warning never blends into the brand.

## Where it's wired

- `os/mkosi/mkosi.postinst.chroot` rebrands os-release (`NAME`/
  `PRETTY_NAME` = "Lisa OS", `ID=lisa`, `LOGO=lisa-logo`, coral
  `ANSI_COLOR`, our URLs).
- `os/mkosi/mkosi.extra/usr/share/icons/hicolor/scalable/apps/lisa-logo.svg`
  — the logo icon the About panel resolves.

This is a first identity pass. Next: GDM/greeter logo, a boot splash
(Plymouth), and a default wallpaper in the same language.
