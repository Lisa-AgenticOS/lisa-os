// Unit tests for the double-tap-Shift state machine (PLAN §5.7.3,
// ADR-0007). Pure standard C++ — compiles and runs on any dev host:
//   c++ -std=c++17 -I../src doubleshift_test.cpp ../src/doubleshift.cpp
// (wired up via tests/CMakeLists.txt and `just ime-test`).

#include "doubleshift.h"

#include <cstdio>

using lisa::DoubleShiftDetector;
using lisa::ShiftKey;

static int failures = 0;

static void expect(bool actual, bool expected, const char *name) {
    if (actual == expected) {
        std::printf("  ok    %s\n", name);
    } else {
        std::printf("  FAIL  %s: expected %s, got %s\n", name,
                    expected ? "fire" : "no fire", actual ? "fire" : "no fire");
        ++failures;
    }
}

// Feed a full bare tap: press at t, release at t+`holdMs`. Returns
// whether the release fired.
static bool tap(DoubleShiftDetector &d, ShiftKey side, uint64_t t,
                uint64_t holdMs = 50) {
    d.onKeyEvent(side, /*isRelease=*/false, /*otherMods=*/false, t);
    return d.onKeyEvent(side, /*isRelease=*/true, /*otherMods=*/false,
                        t + holdMs);
}

int main() {
    // Double-tap fires, exactly on the second release.
    {
        DoubleShiftDetector d;
        expect(tap(d, ShiftKey::Left, 0), false, "first tap alone is silent");
        d.onKeyEvent(ShiftKey::Left, false, false, 200);
        expect(d.onKeyEvent(ShiftKey::Left, true, false, 250), true,
               "double-tap fires on second release");
    }

    // Right Shift works the same, and mixed sides pair up too.
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Right, 0);
        expect(tap(d, ShiftKey::Right, 200), true, "right-side double-tap fires");
    }
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Left, 0);
        expect(tap(d, ShiftKey::Right, 200), true,
               "left-then-right double-tap fires");
    }

    // Shift+letter is typing, not a tap.
    {
        DoubleShiftDetector d;
        d.onKeyEvent(ShiftKey::Left, false, false, 0);   // Shift down
        d.onKeyEvent(ShiftKey::None, false, false, 30);  // 'A' down
        d.onKeyEvent(ShiftKey::None, true, false, 80);   // 'A' up
        expect(d.onKeyEvent(ShiftKey::Left, true, false, 120), false,
               "Shift+letter does not count as a tap");
        expect(tap(d, ShiftKey::Left, 200), false,
               "and the next lone tap is a first tap, not a second");
    }

    // Letter between the two taps resets.
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Left, 0);
        d.onKeyEvent(ShiftKey::None, false, false, 100); // 'x' down
        d.onKeyEvent(ShiftKey::None, true, false, 150);  // 'x' up
        expect(tap(d, ShiftKey::Left, 200), false,
               "intervening key between taps resets");
    }

    // A chord on the SECOND press must not fire either (fire is on
    // release, so Shift-down + letter aborts before anything happens).
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Left, 0);
        d.onKeyEvent(ShiftKey::Left, false, false, 200); // second press in time
        d.onKeyEvent(ShiftKey::None, false, false, 250); // …but then 'A'
        d.onKeyEvent(ShiftKey::None, true, false, 300);
        expect(d.onKeyEvent(ShiftKey::Left, true, false, 350), false,
               "second-press chord (Shift+letter) does not fire");
    }

    // Slow second tap does not fire — but restarts the sequence.
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Left, 0);                        // release at 50
        expect(tap(d, ShiftKey::Left, 451), false,        // press 401ms later
               "second press past 400ms window does not fire");
        expect(tap(d, ShiftKey::Left, 700), true,
               "slow tap became a fresh first tap; next quick tap fires");
    }

    // Boundary: second press exactly at the window edge still fires.
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Left, 0);                        // release at 50
        expect(tap(d, ShiftKey::Left, 450), true,
               "second press at exactly 400ms fires");
    }

    // Triple-tap fires once (1s debounce).
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Left, 0);
        expect(tap(d, ShiftKey::Left, 200), true, "triple-tap: second fires");
        expect(tap(d, ShiftKey::Left, 400), false,
               "triple-tap: third is debounced");
        expect(tap(d, ShiftKey::Left, 600), false,
               "quadruple-tap: still debounced");
        // After the lockout, a fresh double-tap works again.
        tap(d, ShiftKey::Left, 1300);
        expect(tap(d, ShiftKey::Left, 1500), true,
               "after debounce expires, double-tap fires again");
    }

    // Other-modifier chord resets: Ctrl held during a Shift event.
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Left, 0);
        d.onKeyEvent(ShiftKey::Left, false, /*otherMods=*/true, 200);
        expect(d.onKeyEvent(ShiftKey::Left, true, true, 250), false,
               "Ctrl+Shift on second tap does not fire");
    }
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Left, 0);
        d.onKeyEvent(ShiftKey::None, false, false, 100); // Ctrl press = key event
        expect(tap(d, ShiftKey::Left, 200), false,
               "interleaved modifier press between taps resets");
    }

    // Chorded second Shift (hold Left, press Right) is not a tap.
    {
        DoubleShiftDetector d;
        d.onKeyEvent(ShiftKey::Left, false, false, 0);
        d.onKeyEvent(ShiftKey::Right, false, false, 50); // chord → reset
        d.onKeyEvent(ShiftKey::Left, true, false, 100);
        expect(d.onKeyEvent(ShiftKey::Right, true, false, 150), false,
               "chorded both-Shifts does not fire");
    }

    // Key repeat (a press while already down) resets rather than fires.
    {
        DoubleShiftDetector d;
        d.onKeyEvent(ShiftKey::Left, false, false, 0);
        d.onKeyEvent(ShiftKey::Left, false, false, 300); // repeat press
        expect(d.onKeyEvent(ShiftKey::Left, true, false, 350), false,
               "held/repeating Shift does not fire");
    }

    // Stray release with no tracked press is ignored.
    {
        DoubleShiftDetector d;
        expect(d.onKeyEvent(ShiftKey::Left, true, false, 0), false,
               "stray release is ignored");
        tap(d, ShiftKey::Left, 100);
        expect(tap(d, ShiftKey::Left, 300), true,
               "detector still healthy after stray release");
    }

    // External reset() drops an in-flight sequence.
    {
        DoubleShiftDetector d;
        tap(d, ShiftKey::Left, 0);
        d.reset();
        expect(tap(d, ShiftKey::Left, 200), false,
               "reset() forgets the first tap");
    }

    std::printf("fcitx5-lisa doubleshift: %s (%d failure(s))\n",
                failures == 0 ? "all passed" : "FAILED", failures);
    return failures == 0 ? 0 : 1;
}
