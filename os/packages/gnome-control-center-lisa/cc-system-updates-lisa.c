/* Lisa OS updates row for GNOME Settings → System (ADR-0012, ADR-0018).
 *
 * GNOME 50 already reserves a "Software Updates" group on the System
 * page — `software_updates_group` in panels/system/cc-system-panel.blp
 * — but keeps it `visible: false` unless gnome-software or
 * gpk-update-viewer is installed, and its row spawns one of those two
 * binaries. Lisa ships neither: OS updates are whole-image A/B swaps
 * through systemd-sysupdate, driven by `lisa update`.
 *
 * So we claim the group rather than adding a competing one. The System
 * page is where a person looks for "what am I running, is there
 * anything newer", and until now that page answered neither question on
 * Lisa.
 *
 * WHY A SUBPROCESS AND NOT D-BUS. Everything here could be done by
 * talking to org.freedesktop.sysupdate1 directly, and that would avoid a
 * fork. It would also mean a second implementation of "which of running,
 * staged and available is which" — in C, in a GNOME panel, untested —
 * beside the Rust one in `lisa update`. That distinction is exactly what
 * got issue #144 wrong once already, so it lives in one place with unit
 * tests, and this row asks it.
 *
 * WHAT THIS DELIBERATELY DOES NOT DO: install anything. The button
 * checks; it never stages and never reboots. Staging is a privileged,
 * slow, partition-writing operation and it belongs behind the explicit
 * Update control in the Intelligence panel, not behind a button whose
 * label says "check".
 */

/* config.h before gi18n-lib.h: the -lib variant expands _() to
 * dgettext(GETTEXT_PACKAGE, …) and GETTEXT_PACKAGE comes from config.h.
 * This is the pairing panels/system/cc-system-panel.c uses, and this
 * file is compiled into that same panel — the plain <glib/gi18n.h> used
 * by cc-lisa-panel.c would bind these strings to the default domain
 * instead. */
#include <config.h>

#include "cc-system-updates-lisa.h"

#include <glib/gi18n-lib.h>

/* Parsed form of `lisa update --check` output. Every field is optional
 * on purpose: "I could not find out" and "there is nothing" are
 * different answers, and collapsing them is how a machine ends up
 * reporting itself up to date while an update sits staged (#144). */
typedef struct
{
  char *running;
  char *staged;
  char *available;
  gboolean check_failed;
} LisaUpdateState;

static void
lisa_update_state_clear (LisaUpdateState *state)
{
  g_clear_pointer (&state->running, g_free);
  g_clear_pointer (&state->staged, g_free);
  g_clear_pointer (&state->available, g_free);
}

/* `lisa update --check` prints `key: value` lines, omitting any fact it
 * could not establish. Parsing is deliberately tolerant: an unknown key
 * is ignored rather than treated as an error, so adding a field to the
 * CLI later cannot break this row. */
static void
lisa_parse_check_output (const char *out, LisaUpdateState *state)
{
  g_auto (GStrv) lines = NULL;

  if (out == NULL)
    return;

  lines = g_strsplit (out, "\n", -1);
  for (guint i = 0; lines[i] != NULL; i++)
    {
      const char *line = g_strstrip (lines[i]);
      const char *value;

      if ((value = g_str_has_prefix (line, "running:") ? line + 8 : NULL))
        state->running = g_strdup (g_strstrip ((char *) value));
      else if ((value = g_str_has_prefix (line, "staged:") ? line + 7 : NULL))
        state->staged = g_strdup (g_strstrip ((char *) value));
      else if ((value = g_str_has_prefix (line, "available:") ? line + 10 : NULL))
        state->available = g_strdup (g_strstrip ((char *) value));
      else if (g_str_has_prefix (line, "check-failed:") ||
               g_str_has_prefix (line, "note:"))
        state->check_failed = TRUE;
    }
}

/* One sentence for the subtitle, in priority order: the thing that most
 * changes what the person should do next comes first.
 *
 * A staged update outranks an available one because it is the actionable
 * state — the download already happened and only a restart is missing.
 * Reporting "update available" there would invite a second download of
 * something already on disk. */
static char *
lisa_status_text (const LisaUpdateState *state)
{
  const char *running = state->running;

  if (state->staged != NULL)
    return g_strdup_printf (_("%s is ready — restart to apply"), state->staged);

  if (state->available != NULL &&
      (running == NULL || g_strcmp0 (state->available, running) > 0))
    return g_strdup_printf (_("Update available: %s"), state->available);

  if (state->check_failed)
    {
      /* Never render an unreachable channel as "up to date". */
      if (running != NULL)
        return g_strdup_printf (_("%s — could not reach the update channel"),
                                running);
      return g_strdup (_("Could not reach the update channel"));
    }

  if (running != NULL)
    return g_strdup_printf (_("%s — up to date"), running);

  return g_strdup (_("Version unknown"));
}

static void
lisa_check_finished (GObject *source, GAsyncResult *res, gpointer data)
{
  GSubprocess *proc = G_SUBPROCESS (source);
  g_autoptr (AdwActionRow) row = ADW_ACTION_ROW (data); /* ref taken at spawn */
  g_autoptr (GError) error = NULL;
  g_autofree char *out = NULL;
  g_autofree char *text = NULL;
  LisaUpdateState state = { 0 };
  GtkWidget *button;

  if (!g_subprocess_communicate_utf8_finish (proc, res, &out, NULL, &error))
    {
      if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
        return;
      out = NULL;
    }

  lisa_parse_check_output (out, &state);
  /* A CLI that failed to run at all told us nothing — not "no update". */
  if (out == NULL || (state.running == NULL && state.available == NULL))
    state.check_failed = TRUE;

  text = lisa_status_text (&state);
  adw_action_row_set_subtitle (row, text);
  lisa_update_state_clear (&state);

  button = g_object_get_data (G_OBJECT (row), "lisa-check-button");
  if (GTK_IS_BUTTON (button))
    {
      gtk_widget_set_sensitive (button, TRUE);
      gtk_button_set_label (GTK_BUTTON (button), _("Check for Updates"));
    }
}

static void
on_check_clicked (GtkButton *button, gpointer data)
{
  AdwActionRow *row = ADW_ACTION_ROW (data);
  g_autoptr (GError) error = NULL;
  GSubprocess *proc = NULL;

  gtk_widget_set_sensitive (GTK_WIDGET (button), FALSE);
  gtk_button_set_label (button, _("Checking…"));

  proc = g_subprocess_new (G_SUBPROCESS_FLAGS_STDOUT_PIPE |
                             G_SUBPROCESS_FLAGS_STDERR_SILENCE,
                           &error, "lisa", "update", "--check", NULL);
  if (proc == NULL)
    {
      gtk_widget_set_sensitive (GTK_WIDGET (button), TRUE);
      gtk_button_set_label (button, _("Check for Updates"));
      adw_action_row_set_subtitle (row, _("The lisa CLI was not found"));
      return;
    }

  /* The row outlives the call because we hold it: the panel can be
   * navigated away from while the check is in flight, and the callback
   * must not write into freed memory. Released in lisa_check_finished. */
  g_subprocess_communicate_utf8_async (proc, NULL, NULL, lisa_check_finished,
                                       g_object_ref (row));
  g_object_unref (proc);
}

void
lisa_system_updates_attach (AdwPreferencesGroup *group)
{
  g_autofree char *version = NULL;
  GtkWidget *row;
  GtkWidget *button;

  if (!ADW_IS_PREFERENCES_GROUP (group))
    return;

  version = g_get_os_info ("IMAGE_VERSION");

  row = adw_action_row_new ();
  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row), _("Lisa OS"));
  /* The version is shown before any check runs, so the page answers
   * "what am I running" without the user pressing anything. */
  adw_action_row_set_subtitle (ADW_ACTION_ROW (row),
                               version != NULL && *version != '\0'
                                 ? version
                                 : _("Version unknown"));
  adw_action_row_add_prefix (ADW_ACTION_ROW (row),
                             gtk_image_new_from_icon_name ("system-update-symbolic"));

  button = gtk_button_new_with_label (_("Check for Updates"));
  gtk_widget_set_valign (button, GTK_ALIGN_CENTER);
  gtk_widget_set_tooltip_text (button,
                               _("Ask the update channel what is available; "
                                 "downloads nothing"));
  g_signal_connect (button, "clicked", G_CALLBACK (on_check_clicked), row);
  g_object_set_data (G_OBJECT (row), "lisa-check-button", button);
  adw_action_row_add_suffix (ADW_ACTION_ROW (row), button);

  adw_preferences_group_add (group, row);
}
