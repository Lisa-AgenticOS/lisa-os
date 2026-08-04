#!/usr/bin/env python3
"""The dock's pins must be applied, identical on both tracks, and real.

Issue #263: a user reported "assistant doesn't open". The Assistant was
fine — .desktop present, D-Bus service present and activatable, Super+C
bound, launches clean. It was unreachable: not in the dock, and Show
Apps was broken (#262), so the app-grid fallback was gone too.

Underneath that was a second, quieter failure. `favorite-apps` had been
set since 8faf668 — but in the wrong stanza of
`10_lisa-shell.gschema.override`, under
`[org.gnome.settings-daemon.plugins.power]`, a schema with no such key.
glib-compile-schemas warns once during the build and drops the line, so
every image since shipped a dock default that had never been applied.
Measured on the reference device (2026-08-04, image 20260804.76):

    $ GSETTINGS_BACKEND=memory gsettings get org.gnome.shell favorite-apps
    ['org.gnome.Epiphany.desktop', 'org.gnome.Calendar.desktop',
     'org.gnome.Nautilus.desktop', 'org.gnome.Software.desktop',
     'org.gnome.TextEditor.desktop', 'org.gnome.Calculator.desktop']

GNOME's stock list, of which exactly one app is installed — the
one-icon dock the pin was written to fix.

Nothing in the repo could see that: the destination string was right,
the list was right, and the only thing wrong was which group it sat
under. So this check does not read the file as prose. It parses the
override and the dconf database the way their compilers do — group by
group — and asserts:

  * favorite-apps is set under `[org.gnome.shell]` in the package's
    override and under `[org/gnome/shell]` in the image's dconf
    defaults, and nowhere else;
  * the two lists are byte-identical, because Track L gets only the
    override and Track I gets both — two lists one file apart is #239;
  * the Assistant is pinned (the whole of #263);
  * every pinned id resolves to something the machine installs: a
    .desktop the PKGBUILD ships, or a third-party id named below with
    the image package that provides it, which must be in mkosi.conf.

Also asserts the Assistant's activation contract hangs together, since
the dock is now the caller that depends on it: DBusActivatable=true, a
matching [D-BUS Service] Name, and the same id as the GApplication.

Run by `just lint`; costs milliseconds and no package build.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OVERRIDE = ROOT / "os" / "packages" / "lisa" / "10_lisa-shell.gschema.override"
DCONF_DIR = ROOT / "os" / "mkosi" / "mkosi.extra" / "etc" / "dconf" / "db" / "local.d"
PKGBUILD = ROOT / "os" / "packages" / "lisa" / "PKGBUILD"
MKOSI_CONF = ROOT / "os" / "mkosi" / "mkosi.conf"
ASSISTANT = ROOT / "shell" / "assistant"

# Where favorite-apps lives, in each file format's own group syntax.
OVERRIDE_GROUP = "org.gnome.shell"
DCONF_GROUP = "org/gnome/shell"

# Pinned ids Lisa does not build. Each names the package that provides
# it, and that package must be installed by the image — a pin whose app
# is not there renders as a gap, silently.
THIRD_PARTY = {
    "org.gnome.Nautilus.desktop": "nautilus",
    "org.gnome.Console.desktop": "gnome-console",
}


def groups(text, key_pattern):
    """{group: {key: value}} for both .override and dconf keyfiles."""
    out = {}
    group = None
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            group = line[1:-1]
            out.setdefault(group, {})
            continue
        m = re.match(key_pattern, line)
        if m and group is not None:
            out[group][m.group(1)] = m.group(2).strip()
    return out


def find_key(parsed, key):
    """[(group, value)] for every group setting `key`."""
    return [(g, keys[key]) for g, keys in parsed.items() if key in keys]


def parse_list(value):
    return [item.strip() for item in re.findall(r"'([^']*)'", value)]


def desktop_ids_installed_by_package():
    """.desktop basenames the PKGBUILD installs to /usr/share/applications."""
    text = PKGBUILD.read_text()
    return {
        Path(m).name
        for m in re.findall(r"usr/share/applications/([A-Za-z0-9_.-]+\.desktop)", text)
    }


def image_packages():
    return set(re.findall(r"^\s*([a-z0-9][a-z0-9.+-]*)\s*$", MKOSI_CONF.read_text(), re.M))


def check_activation(fail):
    """The Assistant's four names must be one name (#263, #210)."""
    desktop = ASSISTANT / "app.lisaos.Assistant.desktop"
    service = ASSISTANT / "app.lisaos.Assistant.service"
    app_id = desktop.stem
    dtext = desktop.read_text()
    if not re.search(r"^DBusActivatable=true$", dtext, re.M):
        fail(
            f"{desktop.relative_to(ROOT)} does not set DBusActivatable=true, but a "
            f"D-Bus .service ships beside it — a desktop launch will not use it."
        )
        return
    if not service.exists():
        fail(f"{desktop.relative_to(ROOT)} is DBusActivatable with no .service file: "
             f"a click on the icon would call a name nothing starts.")
        return
    m = re.search(r"^Name=(.+)$", service.read_text(), re.M)
    if not m or m.group(1).strip() != app_id:
        fail(f"{service.relative_to(ROOT)} owns '{m.group(1) if m else None}', but "
             f"DBusActivatable derives the name from the file id, '{app_id}'.")
    js = (ASSISTANT / "lisa-assistant.js").read_text()
    if f"'{app_id}'" not in js:
        fail(f"lisa-assistant.js does not use application_id '{app_id}' — the "
             f"activated process would own a different name than the one called.")
    installed = desktop_ids_installed_by_package()
    if desktop.name not in installed:
        fail(f"{desktop.name} is not installed to /usr/share/applications by the PKGBUILD.")
    # The source side, not just the destination string: an install line
    # whose destination reads right and whose source does not exist is a
    # package that fails to build — or, with a typo'd source that happens
    # to exist, a name nothing owns.
    for dest, why in (
        (f"usr/share/dbus-1/services/{service.name}",
         "DBusActivatable would have nothing to activate"),
        (f"usr/share/applications/{desktop.name}",
         "the dock would have no entry to pin"),
    ):
        m = re.search(
            r'install\s+-Dm644\s+(\S+)\s+\\?\s*"\$pkgdir/' + re.escape(dest) + '"',
            PKGBUILD.read_text())
        if not m:
            fail(f"the PKGBUILD installs nothing to /{dest} — {why}.")
        elif not (ROOT / m.group(1)).exists():
            fail(f"the PKGBUILD installs /{dest} from {m.group(1)}, which does "
                 f"not exist in the tree.")


def main():
    problems = []

    def fail(msg):
        problems.append(msg)

    override = groups(OVERRIDE.read_text(), r"([A-Za-z0-9-]+)\s*=\s*(.*)")
    hits = find_key(override, "favorite-apps")
    if not hits:
        fail(f"{OVERRIDE.relative_to(ROOT)} sets no favorite-apps: Track L gets "
             f"GNOME's stock dock, which is Epiphany, Calendar and Software.")
    for group, _ in hits:
        if group != OVERRIDE_GROUP:
            fail(f"{OVERRIDE.relative_to(ROOT)} sets favorite-apps under "
                 f"[{group}], which has no such key. glib-compile-schemas warns "
                 f"once at build time and drops it — the dock default never "
                 f"reaches a device. It belongs under [{OVERRIDE_GROUP}].")

    dconf_hits = []
    for path in sorted(DCONF_DIR.glob("*")):
        if not path.is_file():
            continue
        parsed = groups(path.read_text(), r"([A-Za-z0-9-]+)\s*=\s*(.*)")
        for group, value in find_key(parsed, "favorite-apps"):
            dconf_hits.append((path, group, value))
    if not dconf_hits:
        fail(f"no file in {DCONF_DIR.relative_to(ROOT)} sets favorite-apps: the "
             f"image's dconf defaults carry the app folders and the keys but "
             f"not the dock.")
    for path, group, _ in dconf_hits:
        if group != DCONF_GROUP:
            fail(f"{path.relative_to(ROOT)} sets favorite-apps under [{group}]; "
                 f"dconf reads the group as the settings path, so it belongs "
                 f"under [{DCONF_GROUP}].")

    if hits and dconf_hits:
        a = parse_list(hits[0][1])
        b = parse_list(dconf_hits[0][2])
        if a != b:
            fail("the dock differs between tracks — Track L reads the gschema "
                 f"override, Track I also reads dconf:\n  override: {a}\n  dconf:    {b}")
        if "app.lisaos.Assistant.desktop" not in a:
            fail("the Assistant is not pinned. Lisa is an AI-native OS whose "
                 "defining surface would be invisible on first boot (#263).")
        installed = desktop_ids_installed_by_package()
        packages = image_packages()
        for entry in a:
            if entry in THIRD_PARTY:
                pkg = THIRD_PARTY[entry]
                if pkg not in packages:
                    fail(f"the dock pins {entry}, provided by '{pkg}', which "
                         f"mkosi.conf does not install — the dock renders a gap.")
            elif entry not in installed:
                fail(f"the dock pins {entry}, which the lisa PKGBUILD does not "
                     f"install to /usr/share/applications.")

    check_activation(fail)

    if problems:
        for p in problems:
            print(f"check-dock: {p}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
