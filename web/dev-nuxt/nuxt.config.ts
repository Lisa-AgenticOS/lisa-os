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
      title: 'Lisa OS — developer portal',
      meta: [
        { charset: 'utf-8' },
        { name: 'viewport', content: 'width=device-width, initial-scale=1' },
        { name: 'description', content: 'The Lisa OS developer portal: docs, the API reference (HTTP, D-Bus, MCP), downloads, design guidelines, news, and a live good-first-issues board.' }
      ]
    }
  }
})
