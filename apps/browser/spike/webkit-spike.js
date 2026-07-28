#!/usr/bin/env -S gjs -m
// Phase 0 spike for ADR-0037 / issue #146 — throwaway.
//
// The entire Browser plan rests on one assumption: that GJS can drive
// WebKit-6.0 through GObject introspection well enough to build on. That
// is cheap to test and expensive to be wrong about, so it is tested
// first and alone.
//
// It answers exactly three questions:
//
//   1. Does `WebKit.WebView` instantiate and render a real page from GJS?
//   2. Does `evaluate_javascript()` return a value back INTO GJS? That is
//      the seam the whole extraction design uses instead of a compiled
//      web-process extension (ADR-0037 §2) — if it does not work from
//      GJS, the app stops being pure JS and the plan changes.
//   3. (ANSWERED, 2026-07-29 — kept for the record.) The sandbox is
//      unconditional in WebKit 6.0: set_sandbox_enabled/
//      get_sandbox_enabled do not exist in the API at all, confirmed by
//      grepping WebKit-6.0.gir. Nothing to enable and nothing to check.
//      A container that refuses user namespaces makes bwrap fail loudly
//      at startup, which is the sandbox working, not a fault.
//
// Run on the device:
//     gjs -m apps/browser/spike/webkit-spike.js https://lisaos.dev
//
// Delete this file once Phase 1 exists. It is a probe, not a foundation.

import Gtk from 'gi://Gtk?version=4.0';
import Adw from 'gi://Adw?version=1';
import WebKit from 'gi://WebKit?version=6.0';
import GLib from 'gi://GLib';

const url = ARGV[0] ?? 'https://lisaos.dev';
const results = [];
const record = (q, ok, detail) => {
    results.push(`${ok ? 'PASS' : 'FAIL'}  ${q}${detail ? ` — ${detail}` : ''}`);
    print(`${ok ? 'PASS' : 'FAIL'}  ${q}${detail ? ` — ${detail}` : ''}`);
};

Adw.init();
const app = new Adw.Application({application_id: 'dev.lisaos.BrowserSpike'});

app.connect('activate', () => {
    let view;
    try {
        view = new WebKit.WebView();
        record('Q1a WebKit.WebView constructs from GJS', true);
    } catch (e) {
        record('Q1a WebKit.WebView constructs from GJS', false, `${e}`);
        app.quit();
        return;
    }

    // Q3 is settled by the platform: WebKit 6.0 removed the sandbox
    // toggle entirely, so it is always on. Recorded rather than tested.
    record('Q3 sandbox is unconditional in WebKit 6.0', true, 'no toggle in the API');

    const win = new Adw.Window({
        title: 'Lisa Browser spike',
        default_width: 1024,
        default_height: 720,
    });
    win.set_content(view);
    win.present();

    view.connect('load-changed', (_v, event) => {
        if (event !== WebKit.LoadEvent.FINISHED)
            return;
        record('Q1b a real page finished loading', true, view.get_uri() ?? url);

        // Q2 — the load-bearing one. Extraction (ADR-0037 §2) reads the
        // DOM this way rather than through a compiled .so, so a value
        // has to make it back across into GJS.
        view.evaluate_javascript(
            'JSON.stringify({title: document.title, chars: document.body.innerText.length})',
            -1, null, null, null,
            (v, res) => {
                try {
                    const value = v.evaluate_javascript_finish(res);
                    const json = value.to_string();
                    const parsed = JSON.parse(json);
                    record('Q2 evaluate_javascript returns a value to GJS', true,
                           `title=${JSON.stringify(parsed.title)} text=${parsed.chars} chars`);
                    record('Q2b extracted text is non-empty', parsed.chars > 0,
                           `${parsed.chars} chars`);
                } catch (e) {
                    record('Q2 evaluate_javascript returns a value to GJS', false, `${e}`);
                }
                print('\n--- spike summary ---');
                results.forEach(r => print(r));
                const failed = results.filter(r => r.startsWith('FAIL')).length;
                print(failed === 0
                    ? '\nPhase 0 gate: PASSED — proceed to Phase 1.'
                    : `\nPhase 0 gate: FAILED (${failed}) — see ADR-0037, the WebExtension-on-Zen fallback.`);
                GLib.timeout_add(GLib.PRIORITY_DEFAULT, 500, () => {
                    app.quit();
                    return GLib.SOURCE_REMOVE;
                });
            });
    });

    view.connect('load-failed', (_v, _event, failing, error) => {
        record('Q1b a real page finished loading', false, `${failing}: ${error.message}`);
        app.quit();
        return true;
    });

    view.load_uri(url);
});

app.run([]);
