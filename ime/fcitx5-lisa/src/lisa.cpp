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

#include <atomic>
#include <string>
#include <thread>
#include <utility>

#include <fcitx-config/configuration.h>
#include <fcitx-config/iniparser.h>
#include <fcitx-utils/eventdispatcher.h>
#include <fcitx-utils/i18n.h>
#include <fcitx-utils/key.h>
#include <fcitx/addonfactory.h>
#include <fcitx/addoninstance.h>
#include <fcitx/addonmanager.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/instance.h>

#include "http.h"

namespace {

constexpr char kProofreadPrompt[] =
    "You are a proofreader. Correct spelling, grammar, and punctuation "
    "in the user's text. Preserve its meaning, tone, language, line "
    "breaks, and formatting. Reply with the corrected text only - no "
    "commentary, no quotes.";

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
                                      _("Request timeout (seconds)"), 30};);

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

    fcitx::Instance *instance_;
    LisaConfig config_;
    fcitx::EventDispatcher dispatcher_;
    std::vector<std::unique_ptr<fcitx::HandlerTableEntry<fcitx::EventHandler>>>
        handlers_;
    std::atomic<bool> busy_{false};
};

class LisaWritingToolsFactory final : public fcitx::AddonFactory {
public:
    fcitx::AddonInstance *create(fcitx::AddonManager *manager) override {
        return new LisaWritingTools(manager->instance());
    }
};

} // namespace

FCITX_ADDON_FACTORY_V2(lisa, LisaWritingToolsFactory)
