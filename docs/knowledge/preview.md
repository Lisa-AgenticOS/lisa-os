<!-- GENERATED into the OS knowledge pack from apps/preview/README.md by
     os/repo-tools/build-knowledge.py — edit the source README,
     then regenerate. (#175, ADR-0040) -->

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

### Annotation and page order (slice 2)

For documents the header grows three tools — **Note** (click places a
sticky note, typed into a popover), **Highlight** and **Box** (drag a
marquee) — plus a pages sidebar (thumbnails with move-up/down/remove)
and undo/save. `lib/annotate.js` owns the two coordinate spaces
(poppler renders top-down, annotation rects are bottom-up PDF native;
`annotRect` is the only crossing and it is tested), `lib/reorder.js`
owns the order arithmetic and the qpdf page spec.

Highlights are real PDF text-markup annotations — quadrilaterals built
across the GJS boxed-struct boundary, verified working on the device —
with a square-outline fallback if that marshalling ever breaks. Saving
writes an "(edited)" copy next to the original, never over it; a
changed page order is applied by **qpdf** at save (poppler-glib cannot
reorder or delete pages), staged through a temp file. The saved copy is
then opened, so the window's state is clean and the result is on
screen.

The agent can annotate too: `add_note` and `highlight` are write-tier
tools (top-down page points, 1-based display page). There is
deliberately no save tool — annotations land in the window and the
human decides with Ctrl+S whether they reach disk.

### Text, HTML, and the card (the peek slice)

Space peeks at more than it claims: **text** files (a monospace view;
the extension list in `lib/formats.js`, a binary sniff in `lib/peek.js`
so a `.log` that is actually gzip lands on the card instead of a screen
of mojibake, and a 1 MB display cap that says so out loud), **HTML**
(WebKit, scripts OFF and every navigation except the initial load
refused — a peek, not a browser; without webkitgtk it degrades to
showing the source, labelled), and a **generic card** (icon, name,
type, size) for everything else — because Nautilus sends Space for ANY
selected file and silence reads as broken.

The `.desktop` MIME list deliberately stays images + pdf: registering
text/html would put a peek tool in every Open With list. The previewer
accepts more kinds than the launcher claims, on purpose.

### Media and folders (#200)

Space plays **audio** (flac/m4a/mp3/oga/ogg/opus/wav — bare controls
on a status page) and **video** (m4v/mkv/mov/mp4/webm — `Gtk.Video`,
autoplay, because autoplay IS the Space gesture). Every extension maps
to a GStreamer plugin verified on the image; the GTK media backend is
LINKED INTO libgtk-4 on Arch — there is no module or package for it,
which is why there is deliberately no backend pre-check (a file-probe
for a module dir wrongly declared media unsupported on the device).
If a platform truly lacks a backend, `Gtk.MediaFile` reports it via
`notify::error` and the toast repeats what it said. Space in app
manners is play/pause for media; in quick-look manners it still
closes. **Folders** get the card with a child count (enumeration caps
at 1000 — "1000+ items" — and a capped count drops the size, because
a partial sum presented as THE size is a lie). Transparent images get
a checkerboard baked under them (`composite_color_simple`); the checks
zoom with the image, which is what says "this is transparency".

### Export and signatures (slice 5)

**Export** (Ctrl+E, images and documents): the format menu is the
intersection of `lib/export.js`'s worthwhile five (PNG, JPEG, WebP,
AVIF, TIFF) with what the machine's own pixbuf writers claim — asked
at startup, never assumed. Images convert from the PRISTINE pixels
(the transparency checkerboard is a view aid and never reaches a
file); document pages rasterize at 150 dpi, one page to a chosen
name or every page into a chosen folder, numbered. A same-format
export never suggests the source filename — that invites overwriting
the original.

**Signatures** (documents): draw once in the Sign dialog (strokes,
normalized and stored versioned in
`~/.local/share/lisa/preview/signature.json`), then Sign › Place and
click the page — the scrawl lands as a real `PopplerAnnotStamp` with
a custom image (ink blue-black, rendered at 3× for zoom), so it rides
the existing undo and save-a-copy paths like any other annotation.
Device-verified: a stamped PDF saved, reopened, and rendered with the
signature in place. `lib/signature.js` owns the stroke arithmetic
(empty-canvas saves are no-ops; a degenerate scrawl cannot become a
page-sized stamp).

### Keys

`+` `-` zoom · `0` fit (fills, even for small content) · `1` actual
size · `R` rotate · `←` `→` page (or file, for images) · `[` `]`
previous/next file · `N` note · `H` highlight · `B` box · `P` pages ·
`Ctrl+S` save edited copy · `Ctrl+Z` undo annotation ·
`Ctrl+O` open · `Ctrl+W` close

`Space` depends on how the window opened — Quick Look manners: a
window Space opened is CLOSED by Space (and Escape), the full toggle;
a file opened normally keeps Space as page-forward like any reader.
The distinction exists because this window takes focus, so Nautilus
never gets to send its own close toggle.

Quick-look windows go further (the macOS panel split): they run under
their own app id — `app.lisaos.PreviewPeek`, a NoDisplay .desktop —
so the Lisa dock keeps them out of the running list; they size
themselves to the content (a portrait PDF gets a portrait panel,
capped to most of the monitor); and the header grows an **Open with
Preview** button that hands the file to the real app id (its own dock
presence, app manners) and closes the peek behind it.

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

- **Editing is annotation + page order, not more.** No export or format
  conversion, no signatures, no image annotation (PDFs only), no
  free-text or ink tools — that is slice 3. Highlight marks the dragged
  RECTANGLE; it does not snap to text runs the way selecting text and
  highlighting would.
- **Page reordering needs qpdf at runtime** (in the image package set
  from the slice-2 release onward). Without it, reorder saves refuse
  with the tool named; annotation saves work regardless.
- **Reorder UI is buttons, not drag.** Thumbnail drag-and-drop is a
  follow-up; move-up/move-down/remove do the same job today.
- **Annotating resets rotation.** A rotated view would need a third
  coordinate mapping; picking a tool while rotated snaps back to 0°.
- **No OCR and no vision model.** `read_document` returns PDF text and
  image *metadata*; for an image its `text` is `null` with a note
  saying why, rather than an empty string that reads as "this image
  contains nothing".
- **Tools exist only while a window is open** — mcp-bus defers socket
  activation, deliberately.
- **Space in Files is device-verified** (2026-08-03, a real key press
  through Nautilus's own call path — see the Space section for why the
  bus name matters). The `.desktop` double-click path shipped in
  20260803.66; Space required the versionless-name fix that follows it.

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
