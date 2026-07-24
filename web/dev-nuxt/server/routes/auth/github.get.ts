// GitHub OAuth (nuxt-auth-utils). Reads NUXT_OAUTH_GITHUB_CLIENT_ID /
// NUXT_OAUTH_GITHUB_CLIENT_SECRET from the environment (bp env). Until those
// are set the button will error clearly rather than half-work.
export default defineOAuthGitHubEventHandler({
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
    return sendRedirect(event, '/?error=oauth')
  }
})
