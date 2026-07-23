# Lisa "Intelligence" panel (ADR-0012). Dropped into panels/lisa/ and
# hooked into panels/meson.build's `panels` list by the package's
# prepare() step. Mirrors the stock per-panel build, minus blueprint /
# gresource (the panel builds its UI programmatically) plus json-glib
# (to read `lisa models catalog --json`).

panels_list += cappletname
desktop = 'gnome-@0@-panel.desktop'.format(cappletname)

i18n.merge_file(
  type: 'desktop',
  input: desktop + '.in',
  output: desktop,
  po_dir: po_dir,
  install: true,
  install_dir: control_center_desktopdir
)

sources = files('cc-lisa-panel.c')

cflags += [
  '-DDATADIR="@0@"'.format(control_center_datadir)
]

panels_libs += static_library(
  cappletname,
  sources: sources,
  include_directories: [ top_inc, common_inc ],
  dependencies: common_deps + [ dependency('json-glib-1.0') ],
  c_args: cflags
)
