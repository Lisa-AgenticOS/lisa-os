// Double-tap-Shift detector for fcitx5-lisa (docs/PLAN.md §5.7.3,
// ADR-0007): tap a bare Shift key twice quickly → summon the Lisa
// assistant overlay (dev.lisaos.Overlay1.UI.Summon, ADR-0016 names).
//
// Pure state machine — no fcitx5 types, mirroring http.{h,cpp}: the
// testable half compiles on any dev host (`just ime-test`); the addon
// glue in lisa.cpp translates fcitx KeyEvents into calls here and owns
// the D-Bus side effect.
//
// Rules:
// - A "tap" is press+release of one Shift key (either side) with no
//   other key event and no non-Shift modifier held in between.
// - Two taps fire when the second press lands within `maxIntervalMs`
//   of the first release; detection fires on the second *release*, so
//   Shift+letter chords on the second press never fire.
// - Any non-Shift key event, any non-Shift modifier held, a chorded
//   second Shift while one is down, or a repeat resets to idle.
// - A press slower than the window is not an error: it restarts the
//   sequence as a fresh first tap.
// - After a fire, further activity is ignored for `debounceMs`
//   (triple-tap fires once).

#pragma once

#include <cstdint>

namespace lisa {

// Which Shift key an event is about; None = not a Shift key at all.
enum class ShiftKey { None, Left, Right };

class DoubleShiftDetector {
public:
    struct Config {
        // Max gap between first release and second press.
        uint64_t maxIntervalMs = 400;
        // Post-fire lockout.
        uint64_t debounceMs = 1000;
    };

    DoubleShiftDetector() = default;
    explicit DoubleShiftDetector(Config config) : config_(config) {}

    // Feed every key event (press and release). `key` classifies the
    // event's own key; `otherModifiersHeld` is true when any non-Shift
    // modifier (Ctrl/Alt/Super/…) is down during the event; `nowMs` is
    // any monotonic millisecond clock. Returns true exactly when a
    // double-tap completes (second bare release).
    bool onKeyEvent(ShiftKey key, bool isRelease, bool otherModifiersHeld,
                    uint64_t nowMs);

    // External interruption (config toggled off, focus change, …).
    void reset() { state_ = State::Idle; }

private:
    enum class State { Idle, FirstDown, AwaitSecondPress, SecondDown };

    Config config_{};
    State state_ = State::Idle;
    ShiftKey downSide_ = ShiftKey::None;
    uint64_t firstReleaseMs_ = 0;
    uint64_t lastFireMs_ = 0;
    bool hasFired_ = false;
};

} // namespace lisa
