// Live "good first issue" board from the public GitHub API (no auth needed;
// rate-limited to 60/hr unauthenticated, which is fine at this traffic).
export default defineEventHandler(async (event) => {
  const repo = useRuntimeConfig(event).public.repo
  try {
    const issues = await $fetch<any[]>(`https://api.github.com/repos/${repo}/issues`, {
      params: { labels: 'good first issue', state: 'open', per_page: 8, sort: 'updated' },
      headers: { 'User-Agent': 'lisa-dev-portal', Accept: 'application/vnd.github+json' }
    })
    return (issues || [])
      .filter(i => !i.pull_request)
      .map(i => ({
        number: i.number,
        title: i.title,
        url: i.html_url,
        labels: (i.labels || [])
          .map((l: any) => (typeof l === 'string' ? l : l.name))
          .filter((n: string) => n && n !== 'good first issue')
      }))
  } catch {
    return []
  }
})
