# ADR-0039: The split, and the package index that makes it work

- **Status:** accepted, partially executed — executed through the index
  going live: repos extracted with history, per-repo packages built by CI,
  `[lisa]` hosted, signed, and pacman-verified from a clean machine
  (lisa-os#171). **Step 4 is wired**: `os/mkosi/mkosi.pkgmngr/etc/pacman.d/lisa.conf`
  configures `[lisa]` for the image build and `mkosi.conf` installs
  `lisa-desktop-shell` from it by name, in the line stock `gnome-shell`
  used to occupy. Lisa Desktop is step 4's first consumer, and the only
  one so far — every other Lisa package still arrives through
  release.yml's locally built `PackageDirectories=`, which keeps
  precedence over the index. **Step 5 is started, not finished**: the
  release job now asserts against the mounted image that the shell is
  ours, that stock gnome-shell is absent, that the session is present
  and default, and that the extensions, schemas, dconf defaults and app
  entries are at paths something reads. What it does not assert — and
  cannot from CI — is that a human logs in. Step 6 (removal from the
  monorepo) is untouched.
- Date: 2026-08-02
- Relates: ADR-0006 (monorepo with staged extraction), ADR-0020 (app
  update channel decoupled from the image), ADR-0034 (install/update
  paths take no dependency we do not control), ADR-0038 (Lisa Desktop
  is a hard fork of GNOME Shell), PLAN §3 (packaging economics), PLAN §9
- Extends, does not supersede, ADR-0006. See "What ADR-0006 got right".
- **Claims:**
  - `path:os/mkosi/mkosi.pkgmngr/etc/pacman.d/lisa.conf` — step 4, the image build reads the index
  - `path:shell/launcher` — step 6 is untouched: the surfaces are still in this repo
  - `path:apps/mail` — and so is the app tree

## Context

ADR-0006 was written on 2026-07-21, when the project was mid-M0 with
four Rust crates and stub directories for everything else. It has been
read since as "we are a monorepo", and the question raised on 2026-08-02
was whether it had gone stale.

It had not. It is a **trigger-based** policy — "split by exception, on
triggers, not on a date" — with four named triggers. Precision matters
here, because this ADR is the record: **none of ADR-0006's own four
triggers has fired.** What fired are two triggers it could not have
contained, created by decisions taken after it was written (ADR-0038's
vendored fork; ADR-0020's app channel). The policy held; its trigger
table was incomplete, which is the expected way for a trigger table to
age.

### What ADR-0006 got right

- The mechanism: `git filter-repo`, history preserved, **never
  submodules**. Unchanged, and used here.
- The never-split list: daemons, portal, CLI, `os/*`, `tests/*`, PLAN
  and ADRs. Unchanged. This is the OS, its acceptance gates span these
  components, and they stay single-commit-testable.
- Splitting on a date rather than a trigger is how a one-contributor
  project acquires release engineering it cannot staff.

### What it did not have an answer for

**1. A vendored upstream fork.** ADR-0038 makes Lisa Desktop a hard
fork of GNOME Shell's JavaScript, vendored at a pinned signed tag.
GNOME Shell is not a file we add; it is a codebase with its own history,
its own release cadence, and a scheduled rebase against upstream
forever. Absorbing it into a repo whose CI gates are OS acceptance
blocks means every `just test` carries it and every rebase conflict
lands in the middle of unrelated work. ADR-0006 has no trigger for
this because nothing had been forked yet.

**2. What the extracted piece is consumed AS.** ADR-0006's mechanism
sentence ends: "the monorepo consumes the extracted piece via its
release artifacts (**signed catalog, published crate**), never via git
submodules." Those are the only two artifact kinds it names, because
they were the only two extractions it foresaw. A desktop shell is
neither. An app suite is neither. Extracting them under ADR-0006 as
written leaves the consumption side undefined — which is exactly the
gap that turns a split into a pile of repos nobody can assemble.

The answer is the one PLAN §3 already wrote down and nobody has built:
**a pacman package, in a repo of our own.**

> **Packaging economics:** we inherit ~12k well-maintained packages and
> `pacman`; our delta is a custom repo (~100–200 packages) layered on a
> pinned snapshot mirror of Arch (we control when the base moves, like
> SteamOS's `holo` repo). — PLAN §3

`os/repo-tools/build-packages.sh` builds that `[lisa]` repo today. It
writes to a local directory, the image build consumes it as
`Server = file:///…`, and it has never been hosted anywhere. That was
fine while one repo built one thing. It stops being fine the moment
`shell/` lives somewhere else, because then there is no `file://` that
has both.

## Decision

### 1. Split on two new triggers, recorded in ADR-0006's ledger

| New repo | Contents | Trigger |
|---|---|---|
| `lisa-desktop` | `shell/*`, `ime/fcitx5-lisa` | **New (ADR-0038):** a vendored upstream fork. `ime/` rides along because the IME is part of the desktop surface it summons — ADR-0006 stage 4's own trigger ("upstreaming to fcitx5") has NOT fired. |
| `lisa-apps` | `apps/*` except `apps/notes` | **New (ADR-0020):** apps already have a release channel independent of the image. GJS, no Rust, own cadence. `apps/*` appears in none of ADR-0006's four stages. |
| `lisa-packages` | The `[lisa]` package index: the built repo, the signing key, the publish workflow | **New.** The consumption mechanism ADR-0006 left undefined. |

### 2. ADR-0006's own four stages ALL remain unfired

- **Stage 1, the model catalog** — trigger is "catalog goes live (M1)"
  with its own signed release channel. `models/catalog/` ships inside
  the image; there is no independent catalog release channel yet.
  **Held.**
- **Stage 4, themes / fcitx5 / portal spec** — trigger is a community
  theme engine or actual upstreaming. `ime/fcitx5-lisa` moved to
  `lisa-desktop` as part of the desktop surface, but nothing has been
  upstreamed and no theme engine exists. **Held** (the directory moved;
  the trigger did not fire).
- **Stage 2, `liblisa` SDK** — trigger is "first external consumer /
  crates.io publication". There is no external consumer. `liblisa` is a
  path dependency of `daemons/inferenced` and `cli/lisa`, both of which
  stay; extracting it converts a path dependency into a pinned git
  dependency, and buys a version-bump dance in exchange for nothing.
  **Held.**
- **Stage 3, `lisa_ui`/`lisa_flutter`/`forge`** — trigger is "the
  Flutter lane becomes real (M6)". It is nineteen tracked files and no
  shipped Flutter app. A repo per aspiration is how an org acquires
  eight repos and one working one. **Held.**

Recording a held trigger is the point of a trigger-based policy. These
are not oversights; re-read them when the trigger fires.

### 3. `apps/notes` stays in `lisa-os`

It is the one Rust app, a Cargo workspace member, and it depends on
`libs/mcp-bus` by path. Moving it either drags the workspace across a
repo boundary or converts a path dependency into a git dependency for a
single consumer. It is a seam, it is named here so nobody rediscovers
it, and it moves when there is a second reason to move it.

### 4. Nothing is deleted from `lisa-os` in this pass

The extraction **copies history out**; it does not remove the
directories. `lisa-os` keeps building, keeps testing, and keeps
producing images exactly as it does today, while the new repos come up
alongside. Removal from the monorepo is a separate, explicit step taken
only once the new repo demonstrably produces the package the image
installs.

This is deliberate. A split that removes and adds in one motion has no
moment where you can prove the new path works, because the old one is
already gone.

## The package index

`lisa-packages` is the integration point, and it is what makes "our
package manager make sense": we do not write a package manager, we
**populate one**. pacman is boring technology that already works
(CLAUDE.md rule 4); what has been missing is a place for our packages
to live that is not one developer's `out/` directory.

```
lisa-desktop ─┐
lisa-apps    ─┼─→ each repo builds its own package on tag
lisa-os      ─┘   (PKGBUILD lives with the source it builds)
                        │
                        ▼
              lisa-packages: repo-add into [lisa], sign, publish
                        │
        ┌───────────────┴────────────────┐
        ▼                                ▼
  Track L: pacman -S on stock       Track I: mkosi pulls [lisa]
  Arch/Omarchy (ADR-0003)           at image build (ADR-0001)
```

Three properties this has to hold, each of which is a rule we already
wrote:

- **The PKGBUILD lives with the source it builds.** A packaging recipe
  in a different repo from the code goes stale silently — it builds the
  last thing that worked. `lisa-packages` holds the *index*, not
  everyone's recipes.
- **`[lisa]` is layered on a pinned Arch snapshot** (PLAN §3), so a
  base that moves underneath us is a decision we make, not an event
  that happens to us.
- **ADR-0034 still binds.** "The install, update and recovery paths may
  not depend on infrastructure we do not control." Hosting `[lisa]` on
  GitHub is the same trust we already place in it for release artifacts
  — sysupdate downloads images from there today — so this adds no new
  dependency. What ADR-0034 forbids is a *third-party package manager*
  in those paths, and this is the opposite: it is the first-party
  channel that removes the temptation.

  The mirror-of-last-resort question — what a user does when GitHub is
  unreachable — is real, unanswered, and out of scope here. It is
  tracked, not hand-waved: the A/B image path (ADR-0001) degrades to
  "you keep the working slot you have", which is the property that
  makes deferring this safe.

## Consequences

- Cross-cutting OS work stays atomic, because the never-split list did
  not change. What left is exactly what has its own toolchain and its
  own release cadence.
- Three CIs instead of one, and a package that must be published before
  an image can install it. That is the cost, and it lands on a project
  with one contributor.
- **`lisa-os` gets an integration test it does not have today**: does
  the image built from `[lisa]` still boot with the desktop it expects?
  Under one repo this was structurally guaranteed. It no longer is, and
  ADR-0038 already demands a test that *looks* at the running session.
  That test stops being nice-to-have and becomes the thing standing
  between us and shipping an image with last week's shell in it.
- A version number now means something. Today `build-packages.sh` reads
  the version out of `libs/liblisa/Cargo.toml` — the SDK's version,
  standing in for the whole repo, because there was only one repo.
  Three repos cannot share one crate's version field.

## What would change this

- **If `lisa-desktop`'s CI cannot produce an installable package within
  a couple of weeks**, the split has front-run the work rather than
  followed it, and the extracted repos should sit unpublished until
  ADR-0038 step 2 ("it builds, boots, and CI proves it") is done.
- **If the monorepo directories and the extracted repos both keep
  receiving commits**, the split has failed in the specific way splits
  fail — two sources of truth — and the fix is to complete the removal,
  not to keep syncing.

*Status note, 2026-08-06 (ADR status audit): **amended by ADR-0057**, which
this file had no pointer to. ADR-0057 reverses this ADR's stated intent
that `lisa-desktop` owns the shell surfaces and the IME: the monorepo keeps
them, and `lisa-desktop` narrows in practice to the GNOME Shell fork, until
step 6 happens as a source migration rather than a packaging change. It
also measures the cost of the undone step 6 — 94 paths colliding across
three package pairs in the signed index, none of them declaring
`conflicts` — which is this ADR's own failure clause having fired. One
further drift in the status line above: since ADR-0051 phase 2, the ports
(`lisa-desktop-control-center` among them) no longer arrive through
release.yml's locally built `PackageDirectories=` but prebuilt and
sha256-pinned from the rolling `ports` release.*
