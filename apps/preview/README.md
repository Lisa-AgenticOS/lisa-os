# apps/preview — Preview

Spec: issue #146 (agent surfaces), ADR-0037 (an app the assistant can
see), PLAN §5.8. GJS + GTK4 + libadwaita, app id `app.lisaos.Preview`.

## What it does

Opens images and PDFs, and lets the assistant read what you have open.

It exists because of a gap measured on the reference device on
2026-08-02: **nothing on the system claimed `image/*`**. Double-clicking
a photo in Files did nothing, on a machine whose libraries decode more
image formats than its own browser does — `gdk-pixbuf` reports

```
ani avif bmp dds gif heic icns ico jpeg jxl openexr png pnm qoi
qtif raw svg tga tiff webp xbm xpm
```

while WebKit on the same machine refuses AVIF. The capability was there;
the door was missing.

## How it works

- `lisa-preview.js` — the window. Images render via `GdkPixbuf` →
  `Gdk.Texture`; PDFs via Poppler onto a `Gtk.DrawingArea`. Poppler is
  imported in a `try` and its absence degrades to "images only with a
  message", rather than failing to start on a host without it.
- `lib/formats.js` — pure. What Preview claims (`kindOf`), and folder
  ordering (`siblings`). **`MIME_TYPES` is generated from the same
  list**, and `app.lisaos.Preview.desktop` is generated from that, so
  the app cannot register for a type it cannot open — which is worse
  than not registering, because the file manager then stops offering
  anything else.
- `lib/view.js` — pure. Zoom ladder, fit, fit-width, rotation, page
  stepping. A fixed ladder rather than a multiplier so 100% is always
  exactly reachable; `fitScale` never enlarges (a 16×16 favicon scaled
  to a 4K window is not a fit); `step` clamps rather than wraps, because
  wrapping past the last page reads as "the document reloaded".
- `lib/mcp-protocol.js` — the JSON-RPC surface, pure. **Every result
  carries `provenance: "file"`**, which is *not* `user`: a PDF can
  carry an injection as easily as a web page, and agentd treats
  `Provenance::File` as untrusted, so a Write-tier call downstream
  escalates to a confirmation.
- `lib/mcp.js` — the socket, `$XDG_RUNTIME_DIR/lisa/mcp/<app>.sock`,
  newline JSON-RPC per `libs/mcp-bus`. Same shape as Surfer's on
  purpose; a change to the pattern belongs in both.

### Space, in Files

Select a file in Files and press Space — the macOS Quick Look gesture.
Nautilus does not implement quick preview itself: on Space it calls
whoever owns **`org.gnome.NautilusPreviewer`** — the versionless name;
only the *interface* carries the 2 (nautilus 50.2.2
`src/nautilus-previewer.c:43` appends `PROFILE`, which is empty in
release builds). That is how GNOME Sushi works, and Sushi is **not**
installed on this image, which is why Space did nothing at all before
this app existed.

The first slice shipped owning `org.gnome.NautilusPreviewer2` — wrong,
and invisibly so: Nautilus pings the versionless name at startup, and
when the ping fails it marks the previewer unavailable and drops every
Space press *before making any call*. Nothing reaches any journal. The
fix is the name; the lesson is that the proof must be Nautilus's own
call path, not a hand-built `busctl` call to our own name.

That startup ping auto-starts the service. `--previewer-service` (set
only by the activation file) suppresses the first `activate` present,
so login gets a resident headless previewer instead of an empty window.

`lib/previewer-protocol.js` holds the names and the toggle rule (pure,
tested); `lib/previewer.js` owns the bus name. The signature is not
guessed — `strings /usr/bin/nautilus` shows the variant format `(ssbs)`
beside `ShowFile`, and nautilus's own source builds
`g_variant_new ("(ssbs)", uri, window_handle, close_if_already_visible,
activation_token)`. A D-Bus method with the wrong signature does not
fail softly: the call errors and the key does nothing, which is
indistinguishable from the feature not existing.

`org.gnome.NautilusPreviewer2.service` makes it D-Bus activatable, so
Space works when Preview is closed — which is most of the time, and
exactly when it is wanted. Closing the preview exits the app; the next
Space starts it again.

### Keys

`+` `-` zoom · `0` fit · `1` actual size · `R` rotate ·
`←` `→` page (or file, for images) · `[` `]` previous/next file ·
`Ctrl+O` open · `Ctrl+W` close

## How to extend

Add a tool: implement it in `handlers` (lisa-preview.js), wire the name
in `lib/mcp.js`, declare it in `app.lisaos.Preview.json` with its tier.
The manifest is the catalog — agentd's `ListTools` reads it; there is no
`tools/list` over the wire (checked: nothing in the repo serves one).

Add a format: extend `IMAGE_EXTENSIONS` in `lib/formats.js`. The MIME
list and the `.desktop` regenerate from it. Verify the loader is
actually present by asking `GdkPixbuf.Pixbuf.get_formats()`, never by
grepping a package name or a filename — a `-libs` package installs
libraries that register nothing, and `ls | grep opus` matched an RTP
payloader while no opus *decoder* existed (#146).

## Limits

- **No editing.** No annotation, no page reordering, no export or
  format conversion, no signatures. macOS Preview does all of that;
  this does not, yet. The scope agreed for v1 is full parity, and this
  is the first slice of it — say "images and PDFs open, and the agent
  can read them", not "Preview".
- **No OCR and no vision model.** `read_document` returns PDF text and
  image *metadata*; for an image its `text` is `null` with a note
  saying why, rather than an empty string that reads as "this image
  contains nothing".
- **Tools exist only while a window is open** — mcp-bus defers socket
  activation, deliberately.
- **Not yet exercised through Files' double-click path.** Both halves
  run on the reference device — an image and a PDF were opened and read
  back over the Agent Bus — but the app is not installed there yet, so
  the `.desktop` registration is unproven until a release carries it.

## Footguns, paid for once

- **No top-level `await` in the app module.** The first version imported
  Poppler with `await import()`. The socket bound, accepted connections,
  and never answered one: a top-level await makes the module an async
  evaluation, and `app.run()` then drives the main loop from inside a
  continuation that has not finished, so GIO accepts at the C level
  while the JS `await` on `read_line_async` never resolves. Nothing
  appears in any log. `imports.gi` is synchronous and keeps the module
  plain.
- **`pkill -f lisa-preview` kills your own shell**, because the SSH
  command line contains the string it is matching. Two "the app died"
  investigations were the probe killing itself. Use `pkill -x gjs`.
