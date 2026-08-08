// Lisa Assistant — the modes the navrail switches between.
//
// No GNOME imports: pure data + transforms, so it runs under gjs (the
// app) and jsc (unit tests on any dev host), like the rest of lib/. The
// window owns the widgets; this module owns *what a mode is* and the
// rules that must hold across all of them.
//
// # A mode is a bundle of REAL effects, never a dead button (rule 10)
//
// The navrail is `rail | sidebar (this mode's chats) | chat screen`.
// Each mode changes things that actually happen:
//
//   - `placeholder` — the composer's prompt, tuned to the mode's job;
//   - `wire`        — the `mode` string sent on `Harness1.Run`, so the
//                     daemon can (now or later) select a tool preset and
//                     a policy addendum per mode;
//   - `needsWorkspace` — Code requires a working folder, which is the
//                     existing `workspace` option that turns on the file
//                     tools in harnessd (ADR-0036 §6). This is a mode's
//                     sharpest real behavioural difference and it exists
//                     today.
//
// Conversations are partitioned by mode: the sidebar shows only this
// mode's chats, so a coding session and a research thread do not braid.
//
// # What is client-side today, and what waits on the daemon
//
// `placeholder`, the per-mode chat list, `needsWorkspace`, and putting
// `mode` on the wire are all client-side and land now. The
// DEEPER per-mode behaviour — Code's diff canvas + the best-of-N/verifier
// view (ADR-0065/0067/0069), Research's retrieval + citations, Design's
// artifact panel — needs harnessd to read the `mode` option and vary its
// tools/policy. That is a named follow-up, not shipped here; this module
// puts the `mode` on the wire so the daemon side can be built against a
// contract that already exists rather than inventing one later.

/** The stable ordering the rail renders top-to-bottom. */
export const MODE_IDS = ['chat', 'code', 'design', 'research'];

/**
 * The modes, keyed by id. `icon` names a symbolic in the shipped set.
 * A distinct per-mode accent hue is deliberately NOT carried yet — which
 * mode is active is shown by the rail's selection state; giving each mode
 * its own brand colour is a design decision (and four new tokens under
 * the three-violets rule, ADR-0038) left for later.
 */
export const MODES = {
    chat: {
        id: 'chat',
        label: 'Chat',
        icon: 'user-available-symbolic',
        placeholder: 'Message Lisa…',
        needsWorkspace: false,
        // The general assistant: no workspace, the read-tier bus tools
        // the loop always has, nothing mode-specific asked of the daemon.
        summary: 'A general assistant.',
    },
    code: {
        id: 'code',
        label: 'Code',
        icon: 'text-editor-symbolic',
        placeholder: 'Describe a change, or ask about this folder…',
        needsWorkspace: true,
        // The face of Lisa Coder (ADR-0061): a working folder turns on
        // the file/command tools, and the harness runs its loop over it.
        summary: 'Work in a folder — read, edit, run, verify.',
    },
    design: {
        id: 'design',
        label: 'Design',
        icon: 'applications-graphics-symbolic',
        placeholder: 'Describe something to make or lay out…',
        needsWorkspace: false,
        summary: 'Make and iterate on visual work.',
    },
    research: {
        id: 'research',
        label: 'Research',
        icon: 'system-search-symbolic',
        placeholder: 'Ask a question worth digging into…',
        needsWorkspace: false,
        summary: 'Dig in — multiple sources, cited.',
    },
};

/** The default mode a fresh window opens in. */
export const DEFAULT_MODE = 'chat';

/**
 * A mode by id, falling back to the default rather than throwing — an
 * unknown id (a stale saved preference, a renamed mode) must not brick
 * the window; it lands the person in Chat, the safe general surface.
 * @param {string} id
 */
export function modeById(id) {
    return MODES[id] ?? MODES[DEFAULT_MODE];
}

/**
 * The `mode` option value for `Harness1.Run`. A plain, validated string
 * — never anything the daemon has to parse trust out of; it is a hint
 * for tool/policy selection, and the daemon still decides.
 * @param {string} id
 * @returns {string}
 */
export function wireMode(id) {
    return modeById(id).id;
}

/**
 * Whether switching to `id` requires a working folder before a run can
 * go out. The window uses this to prompt for one on entering Code with
 * none set, rather than letting a run start toolless and confusing.
 * @param {string} id
 */
export function needsWorkspace(id) {
    return modeById(id).needsWorkspace === true;
}
