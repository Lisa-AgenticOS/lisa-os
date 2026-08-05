# Lisa OS websites

- `web/app-nuxt` — **lisaos.app** (marketing). Nuxt 4 + Nuxt UI, statically
  generated, served by nginx. Deploys as bp app `lisa-app`
  (lisa-app.common.al until DNS points the real domain).
- `web/dev-nuxt` — **lisaos.dev** (contributor portal). Nuxt 4 + Nuxt UI +
  nuxt-auth-utils; a Nitro server (GitHub OAuth + the live good-first-issues
  API). Deploys as bp app `lisa-dev`. Needs env (`bp env set lisa-dev …`):
  `NUXT_SESSION_PASSWORD` (set), `NUXT_OAUTH_GITHUB_CLIENT_ID` /
  `NUXT_OAUTH_GITHUB_CLIENT_SECRET` (pending a GitHub OAuth App on the
  Lisa-AgenticOS org; login degrades gracefully until then).
There is exactly ONE tree per site, on purpose. The static predecessors
(`web/app`, `web/dev`) were removed on 2026-08-05: they declared the same
bp app names (`lisa-app`, `lisa-dev`) as the Nuxt trees, so which one
lisaos.app served was decided by whoever deployed last — and nothing in
either tree said which was live. An edit meant for the marketing site
went into the dead file that same day, and was only caught by fetching
the live page and finding it served something else. Recover them from
git history if ever needed; do not restore them as a second tree.

Deploy: `bp deploy web/app-nuxt` / `bp deploy web/dev-nuxt` (server
bp.common.al). A scoped deploy token lives at
`~/.config/basepod/deploy-token` (mode 600, NOT in the repo) for scripted
CI/CD deploys via the HTTP API — prefer it over the account password.
