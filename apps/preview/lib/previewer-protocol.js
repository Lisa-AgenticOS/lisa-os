// Space-to-preview in Files — the macOS Quick Look gesture, via the
// interface Nautilus already calls.
//
// Nautilus does not implement quick preview itself: on Space it calls
// out to whoever owns `org.gnome.NautilusPreviewer2` on the session bus.
// That is how GNOME Sushi works, and Sushi is NOT installed on the Lisa
// image — checked on the reference device, where nothing registered the
// name and Space in Files therefore did nothing at all.
//
// THE SIGNATURE IS NOT GUESSED. A D-Bus method with the wrong signature
// is not a soft failure — the call errors and the key does nothing,
// which looks exactly like the feature not existing. Two independent
// sources agree:
//
//   - the shipped binary: `strings /usr/bin/nautilus` puts the variant
//     format string `(ssbs)` next to `ShowFile`;
//   - nautilus's own source (src/nautilus-previewer.c):
//         g_variant_new ("(ssbs)", uri, window_handle,
//                        close_if_already_visible, activation_token);
//
// THE BUS NAME HAS NO "2" IN IT — only the interface does. This was
// gotten wrong once and cost a silent Space key on the device:
// nautilus 50.2.2 (src/nautilus-previewer.c:43) builds the name as
//     "org.gnome.NautilusPreviewer" PROFILE
// where PROFILE is empty in release builds ("Devel" in devel builds) —
// the versioned name org.gnome.NautilusPreviewer2 is dialed by NOBODY;
// the "2" lives only in the interface. And the failure is invisible by
// design: at startup Nautilus pings the versionless name, and when the
// ping fails it marks the previewer unavailable and every later Space
// press returns before making any call — nothing in any journal.
// Verified on the device 2026-08-03: owning the "2" name, Space
// produced zero bus traffic; owning the versionless one, it activates.

const IFACE_XML = `
<node>
  <interface name="org.gnome.NautilusPreviewer2">
    <method name="ShowFile">
      <arg type="s" name="FileUri" direction="in"/>
      <arg type="s" name="ParentWindowHandle" direction="in"/>
      <arg type="b" name="CloseIfAlreadyVisible" direction="in"/>
      <arg type="s" name="ActivationToken" direction="in"/>
    </method>
    <method name="Close"/>
    <property name="Visible" type="b" access="read"/>
  </interface>
</node>`;

export const BUS_NAME = 'org.gnome.NautilusPreviewer';
export const OBJECT_PATH = '/org/gnome/NautilusPreviewer';
export {IFACE_XML};

/// Decide what a ShowFile call should do, given what is already on
/// screen. Pure, so the toggle rule is testable without a file manager
/// — this module has no gi:// import for exactly that reason, the same
/// split Surfer makes between mcp-protocol.js and mcp.js.
///
/// The rule is Quick Look's: Space on the file you are already
/// previewing closes it; Space on a different file swaps to it. Getting
/// this wrong in the "swap" direction is the annoying one — it closes
/// the window when the user meant to look at the next thing.
export function showFileAction(uri, current, closeIfAlreadyVisible) {
    if (closeIfAlreadyVisible && current.visible && current.uri === uri)
        return {action: 'close'};
    return {action: 'show', uri};
}
