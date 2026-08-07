// The sdk's client edge for the system D-Bus interfaces (ADR-0060).
//
// One table of names and paths, one proxy factory. Before this file,
// every surface declared its own copy of the interface XML and its own
// name/path constants — the drift machine #218 documents. A surface now
// writes:
//
//     import {proxy} from '../lisa_ui/bus/proxy.js';
//     const overlay = proxy('Overlay1');
//     const qid = overlay.AskSync('hello', {})[0];
//
// The XML itself lives in ./interfaces.js, GENERATED from live daemon
// introspection — this file adds only the addressing and the wrapper
// cache.

import Gio from 'gi://Gio';
import {IFACE_XML} from './interfaces.js';
import {BUS} from './addresses.js';

export {BUS};

const wrappers = new Map();

/// The makeProxyWrapper class for an interface, built once per process.
export function proxyClass(iface) {
    if (!IFACE_XML[iface])
        throw new Error(`lisa_ui/bus: unknown interface ${iface}`);
    if (!wrappers.has(iface))
        wrappers.set(iface, Gio.DBusProxy.makeProxyWrapper(IFACE_XML[iface]));
    return wrappers.get(iface);
}

/// A connected proxy on the session bus (synchronous construction, the
/// GJS default). Pass `flags` to defer property loading etc.
export function proxy(iface, {bus = null, flags = null} = {}) {
    const Klass = proxyClass(iface);
    const {name, path} = BUS[iface];
    const args = [bus ?? Gio.DBus.session, name, path];
    if (flags !== null)
        args.push(null, null, flags);
    return new Klass(...args);
}
