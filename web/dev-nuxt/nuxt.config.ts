// Lisa OS contributor portal (lisaos.dev). Nuxt 4 + Nuxt UI + GitHub login.
// Server-rendered (Nitro) — GitHub OAuth needs a server for the token exchange.
export default defineNuxtConfig({
  modules: ['@nuxt/ui', 'nuxt-auth-utils'],
  css: ['~/assets/css/main.css'],
  colorMode: { preference: 'light', fallback: 'light' },
  ssr: true,
  compatibilityDate: '2025-07-01',
  runtimeConfig: {
    // nuxt-auth-utils reads NUXT_OAUTH_GITHUB_CLIENT_ID / _SECRET and
    // NUXT_SESSION_PASSWORD from the environment (set in bp env).
    public: {
      repo: 'Lisa-AgenticOS/lisa-os'
    }
  },
  app: {
    head: {
      htmlAttrs: { lang: 'en' },
      title: 'Lisa OS — build & contribute',
      meta: [
        { charset: 'utf-8' },
        { name: 'viewport', content: 'width=device-width, initial-scale=1' },
        { name: 'description', content: 'Install Lisa OS, read the architecture, and contribute — an OpenAI-compatible endpoint on the machine, a live good-first-issues board, sign in with GitHub.' }
      ]
    }
  }
})
