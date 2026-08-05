// The shipped notifications.conf actually suppresses the nag (#191,
// #201). Pure standard C++ — compiles and runs on any dev host:
//   c++ -std=c++17 -o notifications_test notifications_test.cpp
//   ./notifications_test ../notifications.conf
// (wired up via tests/CMakeLists.txt and `just ime-test`).
//
// WHY A TEST FOR A FIVE-LINE INI FILE. fcitx5 hides a tip when its id
// appears in the notifications addon's `HiddenNotifications` option,
// which fcitx-config marshals as Option<std::vector<std::string>> —
// numbered subkeys under a SECTION:
//
//     [HiddenNotifications]
//     0=wayland-diagnose-gnome
//
// The obvious spelling, `HiddenNotifications=wayland-diagnose-gnome`,
// parses without complaint, unmarshals to an EMPTY vector, and the nag
// survives. So does `HiddenNotifications/0=...`. There is no error, no
// warning, and no log line: the only symptom is a notification on a
// screen nobody in CI is looking at, once per boot, on a machine that
// has to be rebooted to check. That is the exact shape of a regression
// that ships — someone "tidies" the file into flat key=value form and
// the fix silently unfixes.
//
// The device proof lives in the file's own header (both controls, run
// 2026-08-03). This is the part of it a dev host can re-run in 20 ms.

#include <cstdio>
#include <fstream>
#include <string>
#include <vector>

static int failures = 0;

static void expect(bool ok, const char *name) {
    if (ok) {
        std::printf("  ok    %s\n", name);
    } else {
        std::printf("  FAIL  %s\n", name);
        ++failures;
    }
}

static std::string trim(const std::string &s) {
    const char *ws = " \t\r\n";
    const auto b = s.find_first_not_of(ws);
    if (b == std::string::npos) {
        return "";
    }
    return s.substr(b, s.find_last_not_of(ws) - b + 1);
}

int main(int argc, char **argv) {
    const std::string path =
        argc > 1 ? argv[1] : std::string(SOURCE_DIR) + "/../notifications.conf";

    std::ifstream in(path);
    if (!in) {
        std::printf("  FAIL  cannot open %s\n", path.c_str());
        std::printf("fcitx5-lisa notifications: FAILED (1 failure(s))\n");
        return 1;
    }

    // Parse the way fcitx-config does: a [Section] opens a subtree, and
    // numbered keys inside it are the vector's elements.
    std::string line, section;
    std::vector<std::string> hidden;
    bool sawFlatKey = false;   // HiddenNotifications=... at top level
    bool sawSlashKey = false;  // HiddenNotifications/0=...
    bool sawSection = false;

    while (std::getline(in, line)) {
        line = trim(line);
        if (line.empty() || line[0] == '#' || line[0] == ';') {
            continue;
        }
        if (line.front() == '[' && line.back() == ']') {
            section = line.substr(1, line.size() - 2);
            if (section == "HiddenNotifications") {
                sawSection = true;
            }
            continue;
        }
        const auto eq = line.find('=');
        if (eq == std::string::npos) {
            continue;
        }
        const std::string key = trim(line.substr(0, eq));
        const std::string value = trim(line.substr(eq + 1));

        if (section == "HiddenNotifications") {
            // Only numbered keys are elements of the vector.
            bool numeric = !key.empty();
            for (const char c : key) {
                if (c < '0' || c > '9') {
                    numeric = false;
                }
            }
            if (numeric) {
                hidden.push_back(value);
            }
        } else if (section.empty()) {
            if (key == "HiddenNotifications") {
                sawFlatKey = true;
            }
            if (key.rfind("HiddenNotifications/", 0) == 0) {
                sawSlashKey = true;
            }
        }
    }

    expect(sawSection, "declares a [HiddenNotifications] section");
    expect(!sawFlatKey,
           "does NOT use the flat HiddenNotifications= form (unmarshals empty)");
    expect(!sawSlashKey,
           "does NOT use the HiddenNotifications/0= form (unmarshals empty)");
    expect(!hidden.empty(), "the section has at least one numbered entry");

    bool hasGnomeTip = false;
    for (const auto &id : hidden) {
        if (id == "wayland-diagnose-gnome") {
            hasGnomeTip = true;
        }
    }
    // The id is fcitx5's own, from libwayland.so: the GNOME branch of
    // WaylandIMModule's self-diagnose posts showTip("wayland-diagnose-
    // gnome", ...). The sibling ids are wayland-diagnose-kde and
    // wayland-diagnose-other; ours is a GNOME image.
    expect(hasGnomeTip,
           "hides 'wayland-diagnose-gnome' — fcitx5's own tip id for the "
           "\"install the Input Method Panel GNOME Shell Extension\" nag");

    // Numbering starts at 0: fcitx-config reads consecutive indices and
    // stops at the first gap, so a list starting at 1 is an empty list.
    bool startsAtZero = false;
    {
        std::ifstream again(path);
        std::string l, sec;
        while (std::getline(again, l)) {
            l = trim(l);
            if (l.empty() || l[0] == '#' || l[0] == ';') {
                continue;
            }
            if (l.front() == '[' && l.back() == ']') {
                sec = l.substr(1, l.size() - 2);
                continue;
            }
            if (sec == "HiddenNotifications" && l.rfind("0=", 0) == 0) {
                startsAtZero = true;
            }
        }
    }
    expect(startsAtZero, "the list starts at index 0, not 1");

    std::printf("fcitx5-lisa notifications: %s (%d failure(s))\n",
                failures == 0 ? "all passed" : "FAILED", failures);
    return failures == 0 ? 0 : 1;
}
