// Minimal test harness for the shell surfaces' pure-logic modules
// (docs/PLAN.md §11: unit per component). Runtime-agnostic on purpose:
// runs under gjs -m (Linux/CI), node (CI), and jsc -m (macOS dev
// hosts, /System/.../JavaScriptCore.framework/Helpers/jsc) — see
// `just shell-test`. A failing assertion throws; the runner treats a
// non-zero exit as failure.

const log = globalThis.console?.log?.bind(globalThis.console) ?? globalThis.print;

let failures = 0;
let passes = 0;
/// Promises from async test bodies, awaited by `finish`.
const pending = [];

/// Register a test. The body may be sync or async.
///
/// An async body used to pass VACUOUSLY: `fn()` returns a promise, the
/// try block sees nothing thrown, and the test reports "ok" whatever it
/// went on to assert. Under node an unhandled rejection is fatal so the
/// suite at least crashed; under gjs — which `just shell-test` prefers,
/// and which CI therefore uses — it need not be, so an async test could
/// be green and empty. Surfer's suite worked around this by hoisting
/// every `await` to module top level, which is a workaround the harness
/// should not require.
///
/// So a returned promise is collected and awaited by `finish`.
export function test(name, fn) {
    let result;
    try {
        result = fn();
    } catch (e) {
        failures += 1;
        log(`  FAIL  ${name}: ${e.message}`);
        return;
    }
    if (result && typeof result.then === 'function') {
        pending.push(
            result.then(
                () => {
                    passes += 1;
                    log(`  ok    ${name}`);
                },
                (e) => {
                    failures += 1;
                    log(`  FAIL  ${name}: ${e?.message ?? e}`);
                }));
        return;
    }
    passes += 1;
    log(`  ok    ${name}`);
}

export function assertEq(actual, expected, msg = '') {
    const a = JSON.stringify(actual);
    const b = JSON.stringify(expected);
    if (a !== b)
        throw new Error(`${msg} expected ${b}, got ${a}`);
}

export function assert(cond, msg = 'assertion failed') {
    if (!cond)
        throw new Error(msg);
}

/// Await any async tests, then report. `await finish(...)` in a suite
/// that uses async bodies; a sync-only suite may call it without.
export async function finish(suite) {
    if (pending.length > 0)
        await Promise.all(pending);
    log(`${suite}: ${passes} passed, ${failures} failed`);
    if (failures > 0) {
        // Non-zero exit on every supported runtime.
        if (globalThis.imports?.system)
            globalThis.imports.system.exit(1);
        if (globalThis.process?.exit)
            globalThis.process.exit(1);
        throw new Error(`${failures} test(s) failed`);
    }
}
