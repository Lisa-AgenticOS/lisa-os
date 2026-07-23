/* cc-lisa-panel.h — Lisa "Intelligence" panel for GNOME Settings.
 *
 * PLAN §5.3 (Settings panel), §5.11, §8; ADR-0012. Additive downstream
 * panel: id "lisa", registered in shell/cc-panel-loader.c by the
 * gnome-control-center-lisa package's prepare() step.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <shell/cc-panel.h>

G_BEGIN_DECLS

#define CC_TYPE_LISA_PANEL (cc_lisa_panel_get_type ())
G_DECLARE_FINAL_TYPE (CcLisaPanel, cc_lisa_panel, CC, LISA_PANEL, CcPanel)

G_END_DECLS
