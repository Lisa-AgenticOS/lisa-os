// Double-tap-Shift state machine (see doubleshift.h for the rules).
// Pure standard C++ — unit-tested anywhere via `just ime-test`.

#include "doubleshift.h"

namespace lisa {

bool DoubleShiftDetector::onKeyEvent(ShiftKey key, bool isRelease,
                                     bool otherModifiersHeld,
                                     uint64_t nowMs) {
    // Any non-Shift key, or Shift chorded with another modifier,
    // breaks the tap sequence.
    if (key == ShiftKey::None || otherModifiersHeld) {
        state_ = State::Idle;
        return false;
    }

    // Post-fire lockout: a triple (or faster) tap fires once.
    if (hasFired_ && nowMs - lastFireMs_ < config_.debounceMs) {
        state_ = State::Idle;
        return false;
    }

    if (!isRelease) { // Shift press
        switch (state_) {
        case State::Idle:
            state_ = State::FirstDown;
            downSide_ = key;
            break;
        case State::AwaitSecondPress:
            if (nowMs - firstReleaseMs_ <= config_.maxIntervalMs) {
                state_ = State::SecondDown;
            } else {
                // Too slow to pair up — but it is a perfectly good
                // first tap of a new sequence.
                state_ = State::FirstDown;
            }
            downSide_ = key;
            break;
        case State::FirstDown:
        case State::SecondDown:
            // A press while a Shift is already down: chorded second
            // Shift or key repeat — not a tap.
            state_ = State::Idle;
            break;
        }
        return false;
    }

    // Shift release
    switch (state_) {
    case State::FirstDown:
        if (key == downSide_) {
            state_ = State::AwaitSecondPress;
            firstReleaseMs_ = nowMs;
        } else {
            // Released a Shift we never saw pressed cleanly.
            state_ = State::Idle;
        }
        return false;
    case State::SecondDown:
        state_ = State::Idle;
        if (key == downSide_) {
            hasFired_ = true;
            lastFireMs_ = nowMs;
            return true;
        }
        return false;
    case State::Idle:
    case State::AwaitSecondPress:
        // Stray release (held from before a reset) — ignore, and make
        // sure we are idle.
        state_ = State::Idle;
        return false;
    }
    return false;
}

} // namespace lisa
