/* Lisa OS updates row for GNOME Settings → System.
 *
 * Declared separately from cc-system-panel.c so the Lisa delta lives in
 * the Lisa repo, reviewable, instead of inside a sed script in a
 * PKGBUILD. The panel patch is then two lines: include this, and call
 * lisa_system_updates_attach() on the group GNOME already ships.
 */

#pragma once

#include <adwaita.h>

G_BEGIN_DECLS

/* Fill GNOME's (normally hidden) "Software Updates" group with the Lisa
 * OS row: the running version, and a button that checks the update
 * channel. Safe to call with NULL. */
void lisa_system_updates_attach (AdwPreferencesGroup *group);

G_END_DECLS
