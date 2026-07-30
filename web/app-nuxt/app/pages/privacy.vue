<script setup lang="ts">
const repo = 'https://github.com/Lisa-AgenticOS/lisa-os'
const docs = 'https://lisa-dev.common.al'
const updated = '30 July 2026'
useHead({
  title: 'Privacy — Lisa OS',
  meta: [{ name: 'description', content: 'What Lisa OS does with your data: almost nothing, and by construction rather than by promise.' }],
})
</script>

<template>
  <div class="wrap">
    <nav class="nav">
      <NuxtLink to="/" class="brandtext">Lisa OS</NuxtLink>
      <div class="right">
        <a class="ghost" :href="repo">GitHub</a>
        <NuxtLink class="btn line" to="/">Home</NuxtLink>
      </div>
    </nav>

    <article class="legal">
      <span class="eyebrow">Privacy</span>
      <h1>What Lisa OS does with your data.</h1>
      <p class="lede">
        Almost nothing — and where that is true, it is true because of how the
        system is built rather than because we promise it. This page says
        exactly what happens, including the parts where data does leave your
        machine.
      </p>
      <p class="upd">Last updated {{ updated }}</p>

      <h2>The short version</h2>
      <ul>
        <li>Lisa OS runs on your hardware. We operate no servers that receive your files, your mail, or what you say to the assistant.</li>
        <li>The daemons that read your context have <b>no network access at all</b>. That is a property of how they are confined, not a setting.</li>
        <li>Anything that leaves your machine does so because you switched it on, and is recorded in a Ledger you can read.</li>
        <li>We do not sell data, serve advertising, or use your content to train models.</li>
      </ul>

      <h2>The operating system</h2>
      <p>
        Lisa OS collects no analytics and phones nothing home. There is no
        telemetry service, no crash reporter that uploads without asking, and no
        account required to use it. Your files, your mail, your notes and your
        conversations with the local assistant stay on the machine.
      </p>
      <p>
        Local models run on your own hardware. Prompts and results are processed
        on the device and written to the Ledger, which is a file on your disk
        that you own and can delete.
      </p>
      <p>
        <b>Updates</b> are the one thing the base system fetches on its own. To
        download an update, the machine requests release files over HTTPS from
        our release host. That request necessarily reveals your IP address and
        which version you are asking for, the same as any download.
      </p>

      <h2>When something does leave your machine</h2>
      <p>
        Two features send data off the device, and both are off until you turn
        them on:
      </p>
      <ul>
        <li><b>Cloud models.</b> If you sign in to a provider — Anthropic, OpenAI, or another — the text of that conversation goes to that provider, under their privacy policy, not ours. We are not an intermediary: the request goes from your machine to them. Every such call is marked in the interface and recorded in the Ledger.</li>
        <li><b>Mail sync.</b> If you connect a mail account, your machine talks to that mail provider directly, over an encrypted connection, to download your mail onto your own disk. Nothing passes through us.</li>
      </ul>

      <h2>Google user data</h2>
      <p>
        If you connect a Google account, Lisa OS requests access so the mail
        client on your machine can download your mail and, if you enable them,
        your calendar and contacts.
      </p>
      <ul>
        <li>The connection is between <b>your machine and Google</b>. Your mail is never sent to us and never passes through any server we operate.</li>
        <li>Access tokens are stored in the system keyring on your device.</li>
        <li>Mail is stored on your disk in a standard Maildir. Deleting it deletes it.</li>
        <li>Your mail is read on your machine, by software running on your machine. If you have enabled a cloud model and then ask it about your mail, the text you ask about goes to that provider — which is why the system marks it, and why an action taken on a message you did not write is escalated for confirmation.</li>
        <li>You can revoke access at any time in your Google Account's security settings, or by removing the account in Settings. Revoking stops future sync; mail already on your disk stays there until you delete it.</li>
      </ul>
      <p class="callout">
        Lisa OS's use and transfer of information received from Google APIs to
        any other app will adhere to the
        <a href="https://developers.google.com/terms/api-services-user-data-policy">Google API Services User Data Policy</a>,
        including the Limited Use requirements. We do not use Google user data
        for advertising, we do not sell it, we do not allow humans to read it,
        and we do not use it to train any model.
      </p>

      <h2>This website</h2>
      <p>
        These pages are static and carry no analytics, no advertising and no
        third-party trackers. The server that serves them keeps ordinary access
        logs, which include IP addresses, for operating and securing the site.
      </p>

      <h2>Children</h2>
      <p>Lisa OS is not directed at children under 13, and we do not knowingly collect data from them.</p>

      <h2>Changes</h2>
      <p>
        If this policy changes materially we will update the date above and note
        the change in the repository, where its whole history is public.
      </p>

      <h2>Contact</h2>
      <p>
        Questions, or a privacy problem you have found:
        <a :href="repo + '/issues'">open an issue on GitHub</a>. Issues are
        public; for something you would rather not post, use GitHub's private
        vulnerability reporting on the same repository.
      </p>

      <p class="fine">
        Lisa OS is free software under the GPL-2.0 and is supplied without
        warranty. This page describes what the software does; it is not legal
        advice, and it does not replace the licence.
      </p>
    </article>

    <footer>
      <span class="mono">LISA OS · GPL-2.0 · BUILT IN THE OPEN</span>
      <span>
        <a :href="repo">GitHub</a> · <a :href="docs">Docs</a> ·
        <NuxtLink to="/privacy/">Privacy</NuxtLink> ·
        <NuxtLink to="/terms/">Terms</NuxtLink>
      </span>
    </footer>
  </div>
</template>

<style scoped>
.legal { max-width: 46rem; margin: 0 auto; padding: 3rem 0 4rem; }
.legal h1 { margin: .4rem 0 1rem; }
.legal h2 { margin: 2.4rem 0 .6rem; font-size: 1.15rem; }
.legal p, .legal li { line-height: 1.7; }
.legal ul { padding-left: 1.1rem; }
.legal li { margin: .45rem 0; }
.upd { opacity: .6; font-size: .9rem; }
.callout { border-left: 3px solid currentColor; padding-left: 1rem; opacity: .95; }
.fine { margin-top: 2.5rem; font-size: .88rem; opacity: .65; }
.brandtext { font-weight: 600; letter-spacing: .02em; text-decoration: none; }
</style>
