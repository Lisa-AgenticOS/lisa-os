// Live releases from the public GitHub API (no auth; 60/hr unauthenticated
// rate limit is fine at this traffic — same posture as good-first-issues).
export default defineEventHandler(async (event) => {
  const repo = useRuntimeConfig(event).public.repo
  try {
    const releases = await $fetch<any[]>(`https://api.github.com/repos/${repo}/releases`, {
      params: { per_page: 10 },
      headers: { 'User-Agent': 'lisa-dev-portal', Accept: 'application/vnd.github+json' }
    })
    return (releases || []).map(r => ({
      tag: r.tag_name,
      name: r.name || r.tag_name,
      date: r.published_at,
      url: r.html_url,
      prerelease: !!r.prerelease,
      body: r.body || '',
      assets: (r.assets || []).map((a: any) => ({
        name: a.name,
        size: a.size,
        url: a.browser_download_url
      }))
    }))
  } catch {
    return []
  }
})
