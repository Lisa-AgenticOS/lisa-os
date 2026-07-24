// Lisa OS marketing site (lisaos.app). Nuxt 4 + Nuxt UI; statically generated.
export default defineNuxtConfig({
  modules: ['@nuxt/ui'],
  css: ['~/assets/css/main.css'],
  // Light is the default; the toggle switches to dark and is remembered.
  colorMode: { preference: 'light', fallback: 'light' },
  ssr: true,
  compatibilityDate: '2025-07-01',
  app: {
    head: {
      htmlAttrs: { lang: 'en' },
      title: 'Lisa OS — a computer that shows its work',
      meta: [
        { charset: 'utf-8' },
        { name: 'viewport', content: 'width=device-width, initial-scale=1' },
        { name: 'description', content: 'An AI-native Linux desktop. Intelligence built in, running locally by default. Nothing leaves your machine without your say-so — and every model call is written to a record you can read.' },
        { property: 'og:title', content: 'Lisa OS — a computer that shows its work' },
        { property: 'og:description', content: 'An AI-native Linux desktop. Local by default, private by mechanism, everything on the record.' }
      ]
    }
  }
})
