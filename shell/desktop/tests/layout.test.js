// Unit tests for the desktop shell's geometry (ADR-0035).
import {test, assertEq, finish} from '../../testing/harness.js';
import {bottomRightBarriers, bottomRightOf, dockPlacement} from '../lib/layout.js';

test('the hot corner is the monitor\'s far edge, not its size', () => {
    // Primary at the origin.
    assertEq(bottomRightOf({x: 0, y: 0, width: 1920, height: 1080}),
        {x: 1920, y: 1080});
    // A second monitor to the right: the corner must be in GLOBAL
    // coordinates, or the barrier lands on the wrong screen.
    assertEq(bottomRightOf({x: 1920, y: 0, width: 2560, height: 1440}),
        {x: 4480, y: 1440});
});

test('bottom-right barriers run up and left, with both directions flipped', () => {
    const {vertical, horizontal} = bottomRightBarriers({x: 1920, y: 1080}, 32);

    // Vertical barrier: a zero-width segment ON the right edge, running
    // UP from the corner. GNOME's top-left equivalent runs DOWN.
    assertEq(vertical, {
        x1: 1920, x2: 1920, y1: 1048, y2: 1080, direction: 'NEGATIVE_X',
    });

    // Horizontal barrier: zero-height, on the bottom edge, running LEFT.
    assertEq(horizontal, {
        x1: 1888, x2: 1920, y1: 1080, y2: 1080, direction: 'NEGATIVE_Y',
    });
});

test('the barriers are mirror images of GNOME\'s top-left pair', () => {
    // GNOME LTR top-left, verbatim from Shell 50's setBarrierSize:
    //   vertical:   x1=x  x2=x    y1=y  y2=y+size   POSITIVE_X
    //   horizontal: x1=x  x2=x+size  y1=y  y2=y     POSITIVE_Y
    // Mirroring both axes negates every offset and both directions.
    const size = 40;
    const corner = {x: 3000, y: 2000};
    const {vertical, horizontal} = bottomRightBarriers(corner, size);

    assertEq(vertical.y2 - vertical.y1, size, 'vertical barrier length');
    assertEq(horizontal.x2 - horizontal.x1, size, 'horizontal barrier length');
    // Every point of both barriers sits at or inside the corner.
    for (const b of [vertical, horizontal]) {
        assertEq(b.x1 <= corner.x && b.x2 <= corner.x, true, 'x past the corner');
        assertEq(b.y1 <= corner.y && b.y2 <= corner.y, true, 'y past the corner');
    }
    assertEq(vertical.direction, 'NEGATIVE_X');
    assertEq(horizontal.direction, 'NEGATIVE_Y');
});

test('a zero-size corner produces degenerate barriers the caller must skip', () => {
    // GNOME guards with `if (size > 0)`; we keep the same contract
    // rather than inventing a second one.
    const {vertical, horizontal} = bottomRightBarriers({x: 100, y: 100}, 0);
    assertEq(vertical.y1, vertical.y2);
    assertEq(horizontal.x1, horizontal.x2);
});

test('the dock centres on the bottom edge, clear of it', () => {
    const monitor = {x: 0, y: 0, width: 1920, height: 1080};
    assertEq(dockPlacement(monitor, {width: 600, height: 80}, 12),
        {x: 660, y: 988});
});

test('the dock follows the monitor it is on', () => {
    // Second monitor, offset right and down. A placement computed from
    // width/height alone would put the dock on the primary.
    const monitor = {x: 1920, y: 200, width: 1280, height: 800};
    assertEq(dockPlacement(monitor, {width: 400, height: 60}, 10),
        {x: 2360, y: 930});
});

test('a dock wider or taller than the monitor is clamped, never negative', () => {
    const monitor = {x: 0, y: 0, width: 800, height: 600};
    // Off-screen icons are worse than a cramped dock.
    assertEq(dockPlacement(monitor, {width: 1200, height: 80}, 12), {x: 0, y: 508});
    assertEq(dockPlacement(monitor, {width: 400, height: 900}, 12), {x: 200, y: 0});
});

test('odd leftover pixels round rather than producing a fractional position', () => {
    const monitor = {x: 0, y: 0, width: 1001, height: 1000};
    const {x} = dockPlacement(monitor, {width: 100, height: 50}, 0);
    assertEq(Number.isInteger(x), true, 'fractional x blurs the whole dock');
    assertEq(x, 451);
});

finish('desktop/layout');
