// GitHub OAuth (nuxt-auth-utils). Reads NUXT_OAUTH_GITHUB_CLIENT_ID /
// NUXT_OAUTH_GITHUB_CLIENT_SECRET from the environment (bp env). Until those
// are set, degrade gracefully: bounce back to the home page with a flag so
// the UI can say "sign-in is being set up" instead of throwing a 500.
const oauth = defineOAuthGitHubEventHandler({
  config: { emailRequired: false },
  async onSuccess(event, { user }) {
    await setUserSession(event, {
      user: {
        login: user.login,
        name: user.name || user.login,
        avatar: user.avatar_url
      }
    })
    return sendRedirect(event, '/')
  },
  onError(event, error) {
    console.error('GitHub OAuth error:', error)
    return sendRedirect(event, '/?login=error')
  }
})

export default defineEventHandler((event) => {
  const gh = useRuntimeConfig(event).oauth?.github
  if (!gh?.clientId || !gh?.clientSecret)
    return sendRedirect(event, '/?login=unavailable')
  return oauth(event)
})
