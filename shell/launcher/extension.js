// Lisa semantic launcher — GNOME Shell search provider (PLAN §5.7.2).
//
// Augments (not replaces) Shell search: GNOME's own providers keep the
// app-launch lane; this provider adds
//   - "Ask Lisa": the Spotlight-style assistant handoff — every query
//     gets an entry that summons the §5.7.1 overlay with the prompt
//     already submitted (promoted above files when the query reads
//     like a question; routing logic in lib/ranking.js),
//   - calculator/unit answers via qalc (the model routes, it never
//     does arithmetic),
//   - file hits from the Context Fabric (`lisa context search`,
//     FTS5 lexical today; retrieval is ledgered by the CLI).
// Bus actions ("rotate this pdf") join in M5 when lisa-agentd lands;
// semantic vector refinement joins when contextd's embedding pipeline
// lands (§5.3). Budgets (§5.7.2): lexical < 150 ms — FTS5 over a warm
// SQLite plus one subprocess spawn; measured for real by the perf gate
// on reference hardware (§11), not on the dev host.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {
    calculatorExpression, parseQalcOutput, parseContextHits,
    mergeResults, parseResultId,
} from './lib/ranking.js';
import {summonAction} from './lib/summon.js';

Gio._promisify(Gio.Subprocess.prototype, 'communicate_utf8_async');

const MAX_RESULTS = 8;
const FILE_HITS = 5;

// UI-control surface of the assistant overlay (§5.7.1), owned by its
// frontend — see shell/overlay-extension. Kept as literal strings so
// this extension directory stays self-contained.
// The Assistant owns conversations; the Ask lane opens one there
// (#210). GApplication's action surface, so a cold app starts.
const ASSISTANT_BUS = 'app.lisaos.Assistant';
const ASSISTANT_PATH = '/app/lisaos/Assistant';

class LisaSearchProvider {
    constructor(extension) {
        this._extension = extension;
        this.id = 'lisa-launcher';
        this.appInfo = null;         // grid results, no app header
        this.canLaunchSearch = false;
        this.isRemoteProvider = false;
        this._snippets = new Map();  // file path → snippet, for metas
    }

    // ---- Shell search provider API ------------------------------------

    async getInitialResultSet(terms, cancellable) {
        const query = terms.join(' ').trim();
        if (query.length < 2)
            return [];

        const [calc, files] = await Promise.all([
            this._runCalculator(query, cancellable),
            this._searchFiles(query, cancellable),
        ]);

        this._snippets.clear();
        for (const hit of files)
            this._snippets.set(hit.source, hit.snippet);

        return mergeResults({calc, ask: query, files}, MAX_RESULTS);
    }

    getSubsearchResultSet(previousResults, terms, cancellable) {
        // FTS5 is fast enough to re-query rather than filter stale hits.
        return this.getInitialResultSet(terms, cancellable);
    }

    getResultMetas(resultIds, _cancellable) {
        return Promise.resolve(resultIds.map(id => {
            const parsed = parseResultId(id);
            switch (parsed?.kind) {
            case 'calc':
                return {
                    id,
                    name: parsed.result,
                    description: `${parsed.expression} — qalc · Enter copies`,
                    createIcon: size => new St.Icon({
                        icon_name: 'accessories-calculator-symbolic',
                        icon_size: size,
                    }),
                };
            case 'ask':
                return {
                    id,
                    name: `Ask Lisa: “${parsed.query}”`,
                    description: 'Enter opens a new conversation in the Assistant',
                    createIcon: size => new St.Icon({
                        gicon: new Gio.FileIcon({
                            file: this._extension.dir.get_child('lisa-mark.svg'),
                        }),
                        icon_size: size,
                    }),
                };
            case 'file':
                return {
                    id,
                    name: GLib.path_get_basename(parsed.path),
                    description: this._snippets.get(parsed.path) ?? parsed.path,
                    createIcon: size => new St.Icon({
                        gicon: Gio.content_type_get_icon(
                            Gio.content_type_guess(parsed.path, null)[0]),
                        icon_size: size,
                    }),
                };
            default:
                return {id, name: id, description: '', createIcon: () => null};
            }
        }));
    }

    activateResult(resultId, _terms) {
        const parsed = parseResultId(resultId);
        if (parsed?.kind === 'file') {
            const file = Gio.File.new_for_path(parsed.path);
            Gio.AppInfo.launch_default_for_uri(file.get_uri(), null);
            Main.overview.hide();
        } else if (parsed?.kind === 'calc') {
            St.Clipboard.get_default().set_text(
                St.ClipboardType.CLIPBOARD, parsed.result);
            Main.overview.hide();
        } else if (parsed?.kind === 'ask') {
            Main.overview.hide();
            this._summonOverlay(parsed.query);
        }
    }

    // Spotlight handoff: the overview closes and the CONVERSATION
    // opens in the Assistant, in a fresh session (#210).
    //
    // This used to route through the overlay's Summon, which streamed
    // a one-shot answer into a box with nowhere to type a reply — and
    // now that the overlay hands off too, going through it would just
    // flash a window open and closed on the way. So the launcher calls
    // the Assistant directly and the overlay stays what it is: the
    // keyboard-summoned twin of this lane, not a relay.
    //
    // The GAction activates the app if it is not running, which is why
    // this needs no "is it up?" check; a genuine failure (not
    // installed) still tells the user rather than doing nothing.
    _summonOverlay(query) {
        Gio.DBus.session.call(
            ASSISTANT_BUS, ASSISTANT_PATH, 'org.gtk.Actions', 'Activate',
            new GLib.Variant('(sava{sv})',
                ['ask', [new GLib.Variant('s', query)], {}]),
            null, Gio.DBusCallFlags.NONE, 5000, null, (conn, res) => {
                try {
                    Gio.DBus.session.call_finish(res);
                } catch (e) {
                    logError(e, 'assistant handoff failed');
                    Main.notify('Lisa Assistant unavailable',
                        'Could not open a conversation — is the assistant installed?');
                }
            });
    }

    filterResults(results, maxNumber) {
        return results.slice(0, maxNumber);
    }

    // ---- lanes ---------------------------------------------------------

    async _runCalculator(query, cancellable) {
        const expression = calculatorExpression(query);
        if (expression === null)
            return null;
        try {
            const proc = Gio.Subprocess.new(['qalc', '-t', expression],
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
            const [stdout] = await proc.communicate_utf8_async(null, cancellable);
            if (!proc.get_successful())
                return null;
            const result = parseQalcOutput(stdout);
            return result !== null && result !== expression
                ? {expression, result} : null;
        } catch {
            return null; // qalc missing or cancelled — no calc lane.
        }
    }

    async _searchFiles(query, cancellable) {
        try {
            const cli = GLib.getenv('LISA_CLI') ?? 'lisa';
            const proc = Gio.Subprocess.new(
                [cli, 'context', 'search', query, '--limit', String(FILE_HITS)],
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
            const [stdout] = await proc.communicate_utf8_async(null, cancellable);
            if (!proc.get_successful())
                return [];
            return parseContextHits(stdout);
        } catch {
            return []; // index absent or cancelled — other lanes still serve.
        }
    }
}

// The keybinding name in schemas/org.gnome.shell.extensions.lisa-launcher.
// One string, two uses (add + remove), so a rename cannot half-land.
const SUMMON_KEY = 'toggle-search';

export default class LisaLauncherExtension extends Extension {
    enable() {
        this._provider = new LisaSearchProvider(this);
        Main.overview.searchController.addProvider(this._provider);

        // Super+Space, the Spotlight chord (§5.7.2, issue #255). Until
        // this line the launcher was reachable only by opening the
        // overview some other way first: a search provider with no way
        // to summon the search.
        //
        // IGNORE_AUTOREPEAT because holding the chord otherwise fires
        // open/dismiss alternately at the repeat rate.
        this._settings = this.getSettings();
        this._shownId = 0;
        Main.wm.addKeybinding(
            SUMMON_KEY,
            this._settings,
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._summon());
    }

    disable() {
        Main.wm.removeKeybinding(SUMMON_KEY);
        if (this._shownId) {
            Main.overview.disconnect(this._shownId);
            this._shownId = 0;
        }
        this._settings = null;
        if (this._provider) {
            Main.overview.searchController.removeProvider(this._provider);
            this._provider = null;
        }
    }

    _summon() {
        const controller = Main.overview.searchController;
        if (summonAction({
            overviewVisible: Main.overview.visible,
            searchActive: controller?.searchActive === true,
        }) === 'dismiss') {
            Main.overview.hide();
            return;
        }

        if (Main.overview.visible) {
            Main.overview.searchEntry.grab_key_focus();
            return;
        }

        // show() animates, and the controls take key focus as they
        // arrive — so asking now is not enough on its own. Ask now
        // anyway (the entry accepts keystrokes immediately, and a
        // person types before the animation ends), and again once the
        // overview says it is up.
        if (this._shownId === 0) {
            this._shownId = Main.overview.connect('shown', () => {
                Main.overview.disconnect(this._shownId);
                this._shownId = 0;
                Main.overview.searchEntry.grab_key_focus();
            });
        }
        Main.overview.show();
        Main.overview.searchEntry.grab_key_focus();
    }
}
