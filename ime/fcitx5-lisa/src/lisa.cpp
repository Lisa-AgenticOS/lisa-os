// fcitx5-lisa — Writing Tools everywhere, layer 2
// (docs/PLAN.md §5.7.3, ADR-0007).
//
// The input-method trick: IM protocols reach everything that accepts
// text — GTK, Qt, Electron/Chromium, terminals, XWayland — so this
// addon gives every app proofread-on-selection without private toolkit
// hooks. Thin by decree (ADR-0007): key handling + surrounding-text
// capture + commit string here; all model behavior lives in
// lisa-inferenced behind the loopback OpenAI-compat endpoint. Every
// generation is ledgered by the daemon (dataflow rule 4).
//
// v1 behavior: select text in any app, hit the trigger key (default
// Control+Alt+Space) → the proofread text is committed over the
// selection (an IM commit replaces the active selection in standard
// toolkits). The floating compose panel (rewrite menu, "continue
// writing", dictation §5.7.5) grows on this same skeleton.
//
// Second gesture: double-tap a bare Shift key → summon the Lisa
// assistant overlay (dev.lisaos.Overlay1.UI.Summon with an empty
// prompt — ADR-0016 names). The IM layer is the one place that sees
// keys in every app, so the gesture works everywhere. Detection lives
// in doubleshift.{h,cpp} (pure, unit-tested); this file only
// translates KeyEvents and makes the fire-and-forget D-Bus call via
// fcitx5's own dbus addon — asynchronous, never blocking the key path.

#include <atomic>
#include <chrono>
#include <string>
#include <thread>
#include <utility>

#include <fcitx-config/configuration.h>
#include <fcitx-config/iniparser.h>
#include <fcitx-module/dbus/dbus_public.h>
#include <fcitx-utils/dbus/bus.h>
#include <fcitx-utils/dbus/message.h>
#include <fcitx-utils/eventdispatcher.h>
#include <fcitx-utils/i18n.h>
#include <fcitx-utils/key.h>
#include <fcitx/addonfactory.h>
#include <fcitx/addoninstance.h>
#include <fcitx/addonmanager.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/instance.h>

#include "doubleshift.h"
#include "http.h"

namespace {

constexpr char kProofreadPrompt[] =
    "You are a proofreader. Correct spelling, grammar, and punctuation "
    "in the user's text. Preserve its meaning, tone, language, line "
    "breaks, and formatting. Reply with the corrected text only - no "
    "commentary, no quotes.";

// The overlay's UI control surface (see
// shell/overlay-extension/lib/iface.js — the authoritative XML;
// ADR-0016 reverse-DNS names). Summon("", {}) just shows the layer.
constexpr char kOverlayUiBusName[] = "dev.lisaos.Overlay1.UI";
constexpr char kOverlayUiObjectPath[] = "/dev/lisaos/Overlay1/UI";
constexpr char kOverlayUiInterface[] = "dev.lisaos.Overlay1.UI";
// Fire-and-forget: short reply timeout so a wedged frontend can never
// back up into the input path (the call itself is already async).
constexpr uint64_t kSummonTimeoutUsec = 1'000'000; // 1 s

FCITX_CONFIGURATION(
    LisaConfig,
    fcitx::KeyListOption triggerKey{
        this,
        "TriggerKey",
        _("Proofread the current selection"),
        {fcitx::Key("Control+Alt+space")},
        fcitx::KeyListConstrain()};
    fcitx::Option<std::string> host{this, "Host", _("Inference endpoint host"),
                                    "127.0.0.1"};
    fcitx::Option<int> port{this, "Port", _("Inference endpoint port"), 7777};
    fcitx::Option<int> timeoutSeconds{this, "TimeoutSeconds",
                                      _("Request timeout (seconds)"), 30};
    fcitx::Option<bool> doubleShiftSummon{
        this, "DoubleShiftSummon",
        _("Double-tap Shift summons the Lisa assistant overlay"), true};
    // Hands-free (PLAN §5.7.5): the double tap also opens the
    // microphone, the shape people know from Siri. Separate from the
    // summon switch on purpose — somebody may want the overlay on that
    // gesture without it ever listening, and turning a microphone on is
    // not a detail to bundle into somebody else's preference.
    fcitx::Option<bool> doubleShiftListen{
        this, "DoubleShiftListen",
        _("...and starts listening (speak instead of typing)"), true};);

class LisaWritingTools final : public fcitx::AddonInstance {
public:
    explicit LisaWritingTools(fcitx::Instance *instance)
        : instance_(instance) {
        reloadConfig();
        dispatcher_.attach(&instance_->eventLoop());
        handlers_.emplace_back(instance_->watchEvent(
            fcitx::EventType::InputContextKeyEvent,
            fcitx::EventWatcherPhase::PreInputMethod,
            [this](fcitx::Event &event) {
                auto &keyEvent = static_cast<fcitx::KeyEvent &>(event);
                // The double-shift detector sees every key event but
                // never consumes any: Shift taps pass through to the
                // app untouched.
                handleDoubleShift(keyEvent);
                if (keyEvent.isRelease() ||
                    !keyEvent.key().checkKeyList(*config_.triggerKey))
                    return;
                keyEvent.filterAndAccept();
                trigger(keyEvent.inputContext());
            }));
    }

    ~LisaWritingTools() override { dispatcher_.detach(); }

    void reloadConfig() override {
        fcitx::readAsIni(config_, "conf/lisa.conf");
    }

    const fcitx::Configuration *getConfig() const override {
        return &config_;
    }

    void setConfig(const fcitx::RawConfig &raw) override {
        config_.load(raw, true);
        fcitx::safeSaveAsIni(config_, "conf/lisa.conf");
    }

private:
    void trigger(fcitx::InputContext *ic) {
        if (!ic || busy_.exchange(true))
            return;

        std::string selection;
        if (ic->capabilityFlags().test(
                fcitx::CapabilityFlag::SurroundingText) &&
            ic->surroundingText().isValid())
            selection = ic->surroundingText().selectedText();
        if (selection.empty()) {
            // Nothing selected: layer-2 v1 is proofread-on-selection.
            // (The AT-SPI/clipboard fallback is layer 3, PLAN §5.7.3.)
            busy_ = false;
            return;
        }

        auto ref = ic->watch();
        std::string host = *config_.host;
        int port = *config_.port;
        int timeout = *config_.timeoutSeconds;

        // The HTTP round-trip blocks; keep it off the fcitx loop and
        // hop back via the dispatcher for the commit. `ref` guards
        // against the input context dying mid-flight.
        std::thread([this, ref, host, port, timeout,
                     text = std::move(selection)]() {
            std::string payload = lisa::postChatCompletions(
                host, port, lisa::chatRequestBody(kProofreadPrompt, text),
                timeout);
            std::string result = lisa::extractContent(payload);
            dispatcher_.schedule([this, ref, result = std::move(result)]() {
                busy_ = false;
                auto *ic = ref.get();
                if (ic && !result.empty())
                    ic->commitString(result);
            });
        }).detach();
    }

    // Translate the fcitx KeyEvent into the pure detector's event
    // model; on detection, summon the overlay. Rate-limiting (~1 s
    // debounce) lives inside the detector, where it is unit-tested.
    void handleDoubleShift(const fcitx::KeyEvent &keyEvent) {
        if (!*config_.doubleShiftSummon) {
            detector_.reset();
            return;
        }
        const fcitx::Key &key = keyEvent.rawKey();
        lisa::ShiftKey side = lisa::ShiftKey::None;
        if (key.sym() == FcitxKey_Shift_L)
            side = lisa::ShiftKey::Left;
        else if (key.sym() == FcitxKey_Shift_R)
            side = lisa::ShiftKey::Right;
        // Non-Shift modifiers held during the event break the tap
        // (Shift's own bit is irrelevant to bareness; lock states like
        // Caps/Num are ignored on purpose).
        const fcitx::KeyStates states = key.states();
        const bool otherMods = states.test(fcitx::KeyState::Ctrl) ||
                               states.test(fcitx::KeyState::Alt) ||
                               states.test(fcitx::KeyState::Super);
        const auto nowMs = static_cast<uint64_t>(
            std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now().time_since_epoch())
                .count());
        if (detector_.onKeyEvent(side, keyEvent.isRelease(), otherMods, nowMs))
            summonOverlay();
    }

    // Fire-and-forget Summon("", {}) on the session bus via fcitx5's
    // dbus addon (an optional dependency — no bus, no gesture). The
    // async call never blocks input processing; the slot just keeps
    // the pending call alive and is dropped on the next summon.
    // Declared BEFORE summonOverlay(), which calls it, and that order is
    // load-bearing rather than stylistic. The macro defines dbus() with
    // a deduced (auto) return type, and return-type deduction is not
    // deferred for member-function bodies the way name lookup is — a
    // call that appears earlier in the class fails to compile with
    // "use of 'auto ...dbus()' before deduction of 'auto'".
    FCITX_ADDON_DEPENDENCY_LOADER(dbus, instance_->addonManager());

    void summonOverlay() {
        auto *dbusAddon = dbus();
        if (!dbusAddon)
            return;
        fcitx::dbus::Bus *bus = dbusAddon->call<fcitx::IDBusModule::bus>();
        if (!bus)
            return;
        auto msg = bus->createMethodCall(kOverlayUiBusName,
                                         kOverlayUiObjectPath,
                                         kOverlayUiInterface, "Summon");
        msg << std::string(); // empty prompt: just show the layer
        msg << fcitx::dbus::Container(fcitx::dbus::Container::Type::Array,
                                      fcitx::dbus::Signature("{sv}"));
        // options: {"listen": <true>} asks the overlay to open the
        // microphone as well as the layer. Sent only when configured,
        // and the overlay treats its absence as "do not listen" — so an
        // older frontend paired with a newer addon summons silently
        // rather than recording unexpectedly.
        if (*config_.doubleShiftListen) {
            // a{sv} is written as nested containers: a DictEntry inside
            // the array, and the value as an explicit Variant container.
            // There is no DictEntryEnd — every container closes with
            // ContainerEnd, which is why the nesting has to be exact.
            msg << fcitx::dbus::Container(fcitx::dbus::Container::Type::DictEntry,
                                          fcitx::dbus::Signature("sv"));
            msg << std::string("listen");
            msg << fcitx::dbus::Container(fcitx::dbus::Container::Type::Variant,
                                          fcitx::dbus::Signature("b"));
            msg << true;
            msg << fcitx::dbus::ContainerEnd();   // variant
            msg << fcitx::dbus::ContainerEnd();   // dict entry
        }
        msg << fcitx::dbus::ContainerEnd();
        summonSlot_ = msg.callAsync(
            kSummonTimeoutUsec,
            [](fcitx::dbus::Message & /*reply*/) { return true; });
    }


    fcitx::Instance *instance_;
    LisaConfig config_;
    fcitx::EventDispatcher dispatcher_;
    std::vector<std::unique_ptr<fcitx::HandlerTableEntry<fcitx::EventHandler>>>
        handlers_;
    std::atomic<bool> busy_{false};
    lisa::DoubleShiftDetector detector_;
    std::unique_ptr<fcitx::dbus::Slot> summonSlot_;
};

class LisaWritingToolsFactory final : public fcitx::AddonFactory {
public:
    fcitx::AddonInstance *create(fcitx::AddonManager *manager) override {
        return new LisaWritingTools(manager->instance());
    }
};

} // namespace

FCITX_ADDON_FACTORY_V2(lisa, LisaWritingToolsFactory)
