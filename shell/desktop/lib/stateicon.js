// State-dependent app icons — the decisions, with no Shell in it.
//
// The contract (#190, the state half): an app that ships an icon named
// `<icon>-active` in hicolor gets it rendered wherever the shell draws
// its icon WHILE THE APP IS RUNNING — the dock, the overview grid,
// alt-tab. Surfer's meditating robot sits on the beach until the app
// opens; then it surfs. No per-app code in the desktop: the variant's
// existence in the icon theme IS the opt-in.

/// Desktop id -> the active-variant icon name. The id is the .desktop
/// basename; the icon name convention strips that suffix.
export function activeIconName(desktopId) {
    if (typeof desktopId !== 'string' || !desktopId) return null;
    const base = desktopId.endsWith('.desktop')
        ? desktopId.slice(0, -'.desktop'.length) : desktopId;
    return `${base}-active`;
}

/// Where an active variant may live, in lookup order. `dataDirs` is
/// XDG_DATA_DIRS (+ the user data dir first), so existence can be
/// answered with plain file checks — the shell's icon machinery has no
/// "does this themed icon exist" that does not fall back to a generic.
export function candidatePaths(iconName, dataDirs) {
    const out = [];
    for (const dir of dataDirs) {
        for (const size of ['512x512', '256x256', '128x128', 'scalable']) {
            const ext = size === 'scalable' ? 'svg' : 'png';
            out.push(`${dir}/icons/hicolor/${size}/apps/${iconName}.${ext}`);
        }
    }
    return out;
}

/// Whether the shell should swap: only for a RUNNING app with an
/// existing variant. `state` uses Shell.AppState numbering (STOPPED 0,
/// STARTING 1, RUNNING 2) — STARTING keeps the idle icon so a launch
/// that dies never leaves a lying "active" icon behind.
export function shouldUseActive(state, variantExists) {
    return state === 2 && !!variantExists;
}
