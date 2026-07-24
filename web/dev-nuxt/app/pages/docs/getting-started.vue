<script setup lang="ts">
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'Getting started — Lisa OS developers' })
</script>

<template>
  <div>
    <span class="eyebrow">Docs / Getting started</span>
    <h1>Install Lisa OS.</h1>
    <p class="lede">Lisa is an immutable, image-based OS (alpha). Flash the latest USB image and boot it — it runs from the stick, then installs to disk when you're ready. Updates are A/B with boot-counting rollback: a bad boot flips back automatically, and GitHub Releases <em>are</em> the update channel.</p>

    <h2>1. Flash and boot</h2>
    <p>Download <code>lisa-usb-&lt;version&gt;.raw.zst</code> from <NuxtLink to="/downloads">Downloads</NuxtLink> (or the <a :href="`${repo}/releases/latest`">latest GitHub release</a>), then:</p>
    <pre><code><span class="c"># Decompress and write to a USB stick (erases the stick):</span>
zstd -d lisa-usb-*.raw.zst -o lisa.raw
sudo dd if=lisa.raw of=/dev/&lt;your-usb&gt; bs=4M status=progress oflag=sync</code></pre>
    <p>Boot from the stick. The image ships a GNOME desktop with the Lisa shell surfaces — verified on a real 2017 iMac as well as QEMU.</p>

    <h2>2. Install to disk</h2>
    <pre><code><span class="c"># From the booted system — writes the newest release onto the
# internal disk. ERASES IT; asks for typed confirmation first.</span>
lisa install /dev/&lt;internal-disk&gt;</code></pre>
    <p>This is the proto-installer: it streams the latest release onto a whole disk. A guided OOBE installer is milestone M7. Fresh installs get a dedicated <code>/home</code> partition (ADR-0019), so settings and keys survive OS updates.</p>

    <h2>3. Stay updated</h2>
    <pre><code><span class="c"># Pull the newest OS release into the inactive A/B slot
# (systemd-sysupdate) and reboot into it:</span>
lisa update --reboot

<span class="c"># Devices also auto-stage updates via systemd-sysupdate.timer.</span>

<span class="c"># Shell apps update independently of the OS image — minutes,
# no reboot, atomic rollback (ADR-0020):</span>
lisa apps update
lisa apps status
lisa apps rollback</code></pre>
    <div class="note amber">Honest hardening note (from the release notes): sysupdate currently runs with <code>Verify=no</code> — artifact integrity is sha256 via <code>SHA256SUMS</code>; GPG-signed manifests land with the M1 signed repo.</div>

    <h2>First contact with the intelligence</h2>
    <p>Once booted, the system model is an OpenAI-compatible endpoint on loopback — no keys, no cloud:</p>
    <pre><code>lisa ask "write a haiku about entropy"        <span class="c"># streams local tokens</span>
git log | lisa ask "changelog, markdown"      <span class="c"># pipes are context</span>
curl 127.0.0.1:7777/v1/chat/completions ...   <span class="c"># any OpenAI client works</span></code></pre>
    <p>See the <NuxtLink to="/docs/cli">CLI reference</NuxtLink> for every verb and the <NuxtLink to="/api">API reference</NuxtLink> for the HTTP and D-Bus surfaces.</p>

    <h2>Building from source</h2>
    <p>The daemons and CLI are a Rust workspace; <code>just</code> is the umbrella. Requires Rust stable (1.97+).</p>
    <pre><code>git clone {{ repo }}.git &amp;&amp; cd lisa-os
just build   <span class="c"># cargo build --workspace</span>
just test    <span class="c"># cargo test --workspace</span>
just smoke   <span class="c"># end-to-end: daemon + lisa ask</span>
just image   <span class="c"># mkosi OS image — Linux only, normally CI's job</span></code></pre>
    <p>The Rust workspace builds and tests on macOS and Linux; image/systemd/portal work is Linux-only and runs in CI. Two delivery tracks exist (ADR-0003): Track L, a pacman layer on stock Arch/Omarchy (<code>os/layer/</code>), and Track I, the immutable mkosi image (<code>os/mkosi/</code>).</p>
  </div>
</template>
