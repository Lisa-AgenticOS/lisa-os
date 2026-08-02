// The Files previewer bus name — I/O half. The names, the interface XML
// and the toggle rule live in previewer-protocol.js, pure and tested.

import Gio from 'gi://Gio';
import {BUS_NAME, OBJECT_PATH, IFACE_XML, showFileAction} from './previewer-protocol.js';

export {BUS_NAME, OBJECT_PATH};

/// Owns the bus name for as long as the app runs. Callbacks are the
/// app's: `onShow(uri)` and `onClose()`.
export class Previewer {
    constructor({onShow, onClose, isVisible}) {
        this._onShow = onShow;
        this._onClose = onClose;
        this._isVisible = isVisible;
        this._ownerId = 0;
        this._exported = null;
    }

    get Visible() {
        return !!this._isVisible();
    }

    ShowFile(uri, _windowHandle, closeIfAlreadyVisible, _activationToken) {
        const decision = showFileAction(
            uri, {uri: this._currentUri, visible: this.Visible}, closeIfAlreadyVisible);
        if (decision.action === 'close') {
            this._onClose();
            return;
        }
        this._currentUri = uri;
        this._onShow(uri);
    }

    Close() {
        this._onClose();
    }

    start() {
        this._exported = Gio.DBusExportedObject.wrapJSObject(IFACE_XML, this);
        this._ownerId = Gio.bus_own_name(
            Gio.BusType.SESSION, BUS_NAME, Gio.BusNameOwnerFlags.NONE,
            (conn) => this._exported.export(conn, OBJECT_PATH),
            null,
            // Losing the name means something else — Sushi, if it is ever
            // installed — is the previewer now. Say so once rather than
            // fighting for it; two previewers racing on Space is worse
            // than the wrong one winning.
            () => log('lisa-preview: another app owns the Files previewer name'));
    }

    stop() {
        try { this._exported?.unexport(); } catch (e) { /* exiting */ }
        if (this._ownerId) Gio.bus_unown_name(this._ownerId);
    }
}
