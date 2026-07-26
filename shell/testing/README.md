# shell/testing — the GJS test harness

## What it does

`harness.js` is the tiny assert-and-report harness every `shell/*/tests/*.test.js`
imports. The shell surfaces are GJS, so they cannot use `cargo test`; this
is what `just shell-test` runs them through.

## How it works

Tests are plain scripts that import the harness, register cases, and exit
non-zero on failure — no framework, no runner to install, and nothing that
needs network. `just shell-test` executes them with `gjs`, which is why
they work on any dev host including macOS.

## How to extend it

Add `shell/<surface>/tests/<thing>.test.js` and it is picked up by
`just shell-test`. Keep the *logic* being tested in a `lib/` module the
test can import directly — the surfaces are deliberately split into a
headless model plus a thin view for exactly this reason, so a test never
needs a display server or a live D-Bus.

## Limit

These cover model logic, not rendering or D-Bus round-trips. Anything
that needs a real session is verified in CI against the built image
(`screenshots.yml`, the nightly boot checks) or on hardware.
