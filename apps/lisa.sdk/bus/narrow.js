// A NARROW view of a system interface — the sdk's answer to a surface
// that should not be offered everything the daemon serves.
//
// The Assistant's memory pane is why this exists: `MemoryList` and
// `MemoryForget` belong on `dev.lisaos.Harness1`, but a surface with no
// list to show has no business holding a proxy that can forget a
// person's notes. Before this helper the pane kept its own hand-typed
// three-method copy of the interface — the exact drift machine the bus
// module was built to end, kept alive by a good reason.
//
// `narrowIface` cuts the narrow node FROM the sdk's one introspected
// copy, so the subset can never drift from what the daemon actually
// serves: a member that disappears upstream makes this throw at import
// time, in CI, instead of a proxy call failing on a device.
//
// Pure string work, importable from node tests (no gi://).

import {IFACE_XML} from './interfaces.js';

/// The `<node>` XML for `iface`, keeping ONLY the named methods and
/// signals. Throws when a requested member does not exist in the sdk's
/// copy — silence there would hide exactly the drift this prevents.
export function narrowIface(iface, members) {
    const full = IFACE_XML[iface];
    if (!full)
        throw new Error(`lisa.sdk/bus: unknown interface ${iface}`);
    if (!Array.isArray(members) || members.length === 0)
        throw new Error('lisa.sdk/bus: narrowIface needs a non-empty member list');

    const ifaceMatch = full.match(/<interface name="([^"]+)">/);
    const kept = [];
    for (const name of members) {
        // A member name is data, not pattern: escape it, or a name
        // like `Memory.orget` would MATCH MemoryForget instead of
        // throwing — the exact silent-drift this helper exists to
        // refuse (#332).
        if (!/^[A-Za-z][A-Za-z0-9]*$/.test(name))
            throw new Error(
                `lisa.sdk/bus: ${JSON.stringify(name)} is not a plain ` +
                'member name');
        // A member is a <method> or <signal> element; args live inside,
        // so the match runs to the matching close tag. Self-closing
        // members (`<method name="Hide"/>`) match the second arm.
        const m = full.match(new RegExp(
            `[ \\t]*<(method|signal) name="${name}">[\\s\\S]*?</\\1>` +
            `|[ \\t]*<(method|signal) name="${name}"\\s*/>`));
        if (!m)
            throw new Error(
                `lisa.sdk/bus: ${iface} has no member ${name} — the daemon ` +
                'stopped serving it, or the name is wrong');
        kept.push(m[0]);
    }
    return `<node>\n  <interface name="${ifaceMatch[1]}">\n${kept.join('\n')}\n  </interface>\n</node>`;
}
