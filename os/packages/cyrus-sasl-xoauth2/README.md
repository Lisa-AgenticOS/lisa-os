# cyrus-sasl-xoauth2

The SASL mechanism that lets `mbsync` authenticate to Gmail with the
OAuth token GNOME Online Accounts already holds, instead of a password.

## What it does

Adds `XOAUTH2` to the mechanisms Cyrus SASL offers a client, by
installing one shared object into `/usr/lib/sasl2` — the single directory
libsasl scans. That is the whole package: no daemon, no config, no unit.

## Why it exists

Cyrus SASL 2.1.28, which is what Arch ships, offers these and nothing
else:

```
anonymous  crammd5  digestmd5  login  ntlm  plain  sasldb  scram
```

No `XOAUTH2`, no `OAUTHBEARER`. Verified on the field iMac by listing
`/usr/lib/sasl2` and by asking libsasl directly — not assumed from
documentation.

So the chain that turns a connected Google account into mail on disk has
a hole in the middle:

| | |
|---|---|
| Online Accounts holds an OAuth2 token | GNOME |
| `mbsync` turns IMAP into a Maildir | `isync`, in the image |
| **`mbsync` speaks OAuth only through SASL `XOAUTH2`** | **this package** |
| Mail reads the Maildir | `apps/mail` |

Arch does not package it — there is an [open packaging
issue](https://gitlab.archlinux.org/archlinux/packaging/packages/isync/-/work_items/3)
— and it exists in the AUR only, which is not a dependency this OS is
allowed to take: install, update and recovery may not rest on
infrastructure we do not control (ADR-0034). An immutable root also
cannot install one later. So it is built here.

## How it works

A commit-pinned `git+https` source, autotools, one `.so`. The pin is
explicit in `PKGBUILD` (`_commit`) rather than a branch name.

```
makepkg -s          # in an Arch container; the release lane does this
```

`check()` is the part worth knowing about. It compiles
`listmech-check.c`, which asks libsasl what it will actually offer a
client, and runs it twice — once with `SASL_PATH` pointed away from the
build, once at it:

```
  without the plugin: mechs: EXTERNAL
  with the plugin:    mechs: EXTERNAL XOAUTH2
```

The comparison is the point. A file in the right directory is a weaker
claim than a mechanism libsasl loads, and the failure mode being guarded
against is `mbsync` reporting *"unsupported authentication mechanism"*
while `libxoauth2.so` sits there looking installed. A probe that reports
"found" without ever having reported "not found" proves nothing, so the
control run is inside the check and the build fails if it ever starts
succeeding for the wrong reason.

That control earned its place immediately: the first version of the probe
reported `EXTERNAL ANONYMOUS` from **both** runs. Not a missing plugin —
a missing callback. libsasl will not offer a mechanism it cannot feed
credentials to, so `PLAIN` and `LOGIN` were invisible too, and a probe
with no callbacks makes a working plugin look absent.

## Extending it

Re-pinning is editing `_commit` and `pkgver`'s date. Upstream last moved
in August 2021, and that is less alarming than it looks: XOAUTH2 is a
frozen wire format — `user=<addr>^Aauth=Bearer <token>^A^A`, base64'd —
specified by Google, not by this repository. A stale pin here is a stable
one. A security fix would still matter, which is why the pin is greppable
rather than a moving branch.

**If Arch ever ships this, delete this directory** and name the official
package in `os/mkosi/mkosi.conf` instead. Same package name, so nothing
else changes.

## Limits

- **x86_64 and aarch64 are declared; only x86_64 is built today.** The
  release lane builds this package; the aarch64 image lane has its own
  `Packages=` block and does not yet include it.
- **It authenticates; it does not configure.** Writing the `mbsyncrc`
  that names the mechanism, and fetching a fresh token for `PassCmd`, is
  `lisa mail setup` (issue #155). This package on its own changes
  nothing a user can see.
- **No server-side use.** The plugin builds both halves; only the client
  mechanism is ever exercised here.
