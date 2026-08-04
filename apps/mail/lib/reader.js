// The reading pane's shape, and who owns the Space key (#247).
//
// No GTK imports on purpose: `lisa-mail.js` needs a GTK host to run at
// all, so anything expressed as a sequence of `append()` calls in there
// is untestable by construction. The two things #247 was actually about
// — where the attachment bar sits, and what Space does — are expressed
// here as data and a pure function, and `lisa-mail.js` is built from
// them.

/// The reading pane's children, top to bottom.
///
/// Attachments go BELOW the body. They used to sit between the sender
/// and the message, which is where no mail client puts them and which
/// pushed the message itself down the pane.
///
/// ## Pinned, not scrolling — and it is not really a choice
///
/// "Below the body" could mean inside the scrolled region, so the bar
/// scrolls away on a long message, or pinned under it so it is always
/// reachable. Pinned, for two reasons, and the second is the decisive
/// one:
///
/// 1. A message that says "see attached" can be forty screens long. An
///    attachment bar you have to scroll to the bottom to reach is a bar
///    you will not find, and the whole point of moving it was to put it
///    where people look.
/// 2. **Inside the scrolled region is not uniformly available.** The
///    body is a `Gtk.Stack` of two renderers: a `Gtk.TextView` in a
///    `Gtk.ScrolledWindow` for plain mail, and a `WebKit.WebView` for
///    HTML. You cannot append a GTK widget into a WebView's scrolled
///    content, so "inside the scroll" would give plain-text mail one
///    behaviour and HTML mail another — the attachment bar moving
///    depending on how the sender composed the message.
///
/// Pinning costs nothing structurally: the reader's container is a plain
/// vertical box that is not itself scrolled, and only the body slot
/// expands, so a child appended after the body simply sits under it.
export const READER_ORDER = Object.freeze([
    'title',
    'from',
    // Directly above the body: it describes what was withheld from
    // THIS message's rendering, so it belongs with the thing it is
    // talking about.
    'remote-banner',
    'body',
    'attachments',
]);

/// The one slot that expands to eat the leftover height. Exactly one
/// may, and it must be the body — that is what makes everything after
/// it pinned rather than floating in the middle of a short message.
export const READER_EXPANDING_SLOT = 'body';

/// Map `{slot: widget}` onto `READER_ORDER`, in order.
///
/// Throws on a slot with no widget rather than skipping it. #247 began
/// with an attachment bar that was built and wired and then not where
/// anyone expected it; a bar that silently stops being appended is the
/// same defect with a quieter failure, so it is made loud.
export function orderedReaderChildren(widgets) {
    return READER_ORDER.map(slot => {
        const widget = widgets?.[slot];
        if (widget === undefined || widget === null)
            throw new Error(`the reader slot "${slot}" has no widget`);
        return widget;
    });
}

/// What the Space key should do inside the attachment list.
///
/// ## Which widget owns Space, exactly
///
/// The controller that calls this is on the attachment `Gtk.ListBox`, so
/// it is only ever on the event's path while focus is INSIDE that list.
/// A press in the message list, in the search entry or in any text field
/// never reaches it, and Space keeps doing what it does everywhere else
/// in the window. That is a property of *where the controller is added*,
/// not of this function — no unit test can observe it, so do not move
/// the `add_controller` call without re-reading this paragraph.
///
/// The controller is in CAPTURE phase deliberately: `Gtk.ListBox` binds
/// Space to its own cursor-row handling, and a bubble-phase controller
/// would arrive after the list had already consumed the key. Capturing
/// means every exception has to be made explicitly rather than
/// inherited — hence `focusIsButton`.
///
/// Returns `{action, attachment}`:
///   {action: 'peek', attachment} — preview that attachment.
///   {action: 'pass'}             — let the key through untouched.
///
/// It returns WHICH attachment rather than just "yes": the caller must
/// not re-derive that. An earlier version answered 'peek'/'pass' and
/// left `lisa-mail.js` to pick the attachment with its own copy of the
/// precedence rule — so swapping the precedence here changed nothing
/// any test could see, and a mutation proved it by surviving. One
/// decision, in one place, returned whole.
export function spaceAction({
    focusIsButton = false,
    focusedAttachment = null,
    selectedAttachment = null,
} = {}) {
    // A focused Save/Open button keeps Space: GtkButton binds it to
    // activate, and stealing it would break the keyboard path to the two
    // actions in the row.
    if (focusIsButton)
        return {action: 'pass'};
    // The row the person is on: the focused one, or the selected one
    // when focus is on the list itself. Focus wins — it is where the
    // person is now, and the selection may be where they were.
    const attachment = focusedAttachment ?? selectedAttachment;
    return attachment ? {action: 'peek', attachment} : {action: 'pass'};
}
