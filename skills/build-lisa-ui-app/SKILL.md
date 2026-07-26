---
name: build-lisa-ui-app
description: Build, verify and install a Flutter app for Lisa using the lisa_ui widget kit
tools: read_file, write_file, edit_file, list_files, grep, run_command
---

# Build a lisa_ui Flutter app

Apps on Lisa are Flutter apps written against **lisa_ui**. Four steps:
scaffold, write, verify, install.

## 1. Scaffold

```sh
lisa forge --flutter --project ./tip-calc "a tip calculator"
```

That creates `pubspec.yaml` (with a path dependency on lisa_ui),
`lib/main.dart`, and `test/smoke_test.dart`, then runs `flutter pub get`.
The Dart package is named after the project directory (`tip-calc` →
`tip_calc`), which is also the built binary's name.

The SDK comes from `lisa forge --setup` (pinned Flutter, installed to
`/var/lib/lisa/flutter`); a `flutter` already on `PATH` wins.

## 2. Write against lisa_ui — one import, never Material directly

```dart
import 'package:lisa_ui/lisa_ui.dart';
```

**Never `import 'package:flutter/material.dart'` in app code.** lisa_ui
re-exports the curated Material vocabulary plus Lisa theming and the
AI-native widgets, and phase 2 swaps the backend to a vendored fork with
no app-facing API change (ADR-0014). An app that imports Material
directly breaks on that swap.

Start from `LisaApp` + `LisaScaffold`:

```dart
void main() => runApp(
  const LisaApp(
    title: 'Tip Calculator',
    home: LisaScaffold(title: 'Tip Calculator', body: Body()),
  ),
);
```

Beyond Material: `LisaCard`, `LisaStreamText` (token-by-token model
output with a stop affordance and a provenance footnote row), and
`ConsentChip` (states the scope plainly, allow/deny, no dark patterns).
Read tokens with `LisaTheme.of(context)` — never hardcode color, radius
or spacing.

If a widget you need is missing from the re-export list, add it to
`libs/lisa_ui/lib/lisa_ui.dart`'s `show` clause; do not reach around the
kit.

## 3. Verify

```sh
flutter analyze --no-pub
flutter test
```

This is what `lisa forge --flutter` runs as its verifier: the loop is not
done until analysis is clean. A missing symbol usually means it is not in
lisa_ui's `show` clause yet — check there before assuming a typo.

## 4. Install and run it

```sh
lisa forge --run --project ./tip-calc     # --build to install without launching
```

That generates the Linux runner from the SDK template if it is missing,
runs `flutter build linux --release`, installs the bundle under the forge
apps directory (`/var/lib/lisa/forge/apps/<app-id>/bundle`, or
`~/.local/share/lisa/forge/apps/...` on a machine without the system
one), and writes a `.desktop` entry so the app appears in the app grid.
The previous build is kept beside it as `bundle.previous`.

A Linux build needs `clang`, `cmake`, `ninja`, `pkg-config` and `gtk3` on
the device.
