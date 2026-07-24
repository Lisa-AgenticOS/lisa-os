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
- `web/app`, `web/dev` — the retired static predecessors; superseded by the
  Nuxt builds, kept until the swap has soaked.

Deploy: `bp deploy web/app-nuxt` / `bp deploy web/dev-nuxt` (server
bp.common.al). A scoped deploy token lives at
`~/.config/basepod/deploy-token` (mode 600, NOT in the repo) for scripted
CI/CD deploys via the HTTP API — prefer it over the account password.
