// The start page (#182): local bytes, zero network, one input.
//
// The page cannot call into GJS, so the form navigates to a private
// scheme (`lisa-go:?q=…`) that the app intercepts in decide-policy and
// routes through resolveInput — the SAME brain as the address bar, so
// "words search, addresses navigate" holds in both places and the
// refusal list is never re-implemented (the #166 rule again).

/// Where new tabs go.
export const START_URI = 'lisa://start';

/// The navigation the form emits; the app intercepts anything with
/// this prefix and never lets the engine see it.
export const GO_PREFIX = 'lisa-go:';

/// The text a lisa-go: navigation carries, or null when it is not one.
export function goQuery(uri) {
    const u = String(uri ?? '');
    if (!u.startsWith(GO_PREFIX))
        return null;
    const q = /[?&]q=([^&]*)/.exec(u);
    return q ? decodeURIComponent(q[1].replace(/\+/g, ' ')) : '';
}

/// Self-contained: inline styles from branding tokens, no fetches.
export const START_PAGE_HTML = `<!doctype html>
<html><head><meta charset="utf-8"><title>New Tab</title>
<style>
  html, body {
    height: 100%; margin: 0;
    background: linear-gradient(160deg, #4F378B 0%, #0F172A 70%); /* tokens: violet-700, dark-base */
    font-family: Rubik, sans-serif;
    display: flex; align-items: center; justify-content: center;
  }
  form { width: min(560px, 82vw); text-align: center; }
  h1 {
    color: #F8FAFC; /* token: warm-white */
    font-weight: 600; letter-spacing: .04em; margin: 0 0 28px;
    font-size: 28px;
  }
  h1 span { color: #9B7BE8; } /* token: violet-300 */
  input {
    width: 100%; box-sizing: border-box;
    font: inherit; font-size: 17px;
    color: #F8FAFC;
    background: rgba(248, 250, 252, 0.09);
    border: 1px solid rgba(248, 250, 252, 0.16);
    border-radius: 14px;
    padding: 14px 20px;
    outline: none;
  }
  input:focus { border-color: #9B7BE8; } /* token: violet-300 */
  input::placeholder { color: rgba(248, 250, 252, 0.45); }
</style></head>
<body>
  <form action="lisa-go:submit" method="get">
    <h1>Sur<span>fer</span></h1>
    <input name="q" autofocus autocomplete="off"
           placeholder="Search or enter address">
  </form>
</body></html>`;
