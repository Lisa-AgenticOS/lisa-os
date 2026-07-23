/* cc-lisa-panel.c — Lisa "Intelligence" panel for GNOME Settings.
 *
 * PLAN §5.3 (Settings panel), §5.11, §8; ADR-0012.
 *
 * v1 content:
 *   - Local models: `lisa models catalog --json` (§8 hardware-aware
 *     fit), each row badged by what runs on THIS machine, with a
 *     one-click Get (`lisa models get`) for pinned models that fit.
 *     Local inference never leaves the machine — nothing here is
 *     egress-marked.
 *   - Providers & privacy: opens the org.lisa.Settings app for the full
 *     provider/key/OAuth + offload-consent flow (native in v2).
 *
 * Programmatic UI (no .ui/gresource): CcPanel derives AdwNavigationPage,
 * so we set an AdwToolbarView + AdwPreferencesPage as its child.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include <adwaita.h>
#include <gio/gdesktopappinfo.h>
#include <glib/gi18n.h>
#include <json-glib/json-glib.h>

#include "cc-lisa-panel.h"

struct _CcLisaPanel
{
  CcPanel parent_instance;

  AdwPreferencesPage  *page;
  AdwPreferencesGroup *models_group;   /* rebuilt on every refresh */
  GCancellable        *cancellable;
};

CC_PANEL_REGISTER (CcLisaPanel, cc_lisa_panel)

/* ------------------------------------------------------------------ */
/* Local models                                                        */
/* ------------------------------------------------------------------ */

static void refresh_models (CcLisaPanel *self);

static gchar *
model_subtitle (JsonObject *m)
{
  g_autoptr (GString) s = g_string_new (NULL);
  const gchar *task = json_object_get_string_member_with_default (m, "task", NULL);
  const gchar *license = json_object_get_string_member_with_default (m, "license", NULL);
  gint64 ram = json_object_get_int_member_with_default (m, "min_ram_gb", 0);

  if (task && *task)
    g_string_append (s, task);
  if (license && *license)
    g_string_append_printf (s, "%s%s", s->len ? " · " : "", license);
  if (ram > 0)
    g_string_append_printf (s, "%sneeds ~%" G_GINT64_FORMAT " GiB", s->len ? " · " : "", ram);

  return g_string_free (g_steal_pointer (&s), FALSE);
}

/* Plain-words badge + a style class, mirroring the GJS view-model. */
static const gchar *
model_badge (JsonObject *m, const gchar **css_class)
{
  const gchar *fit = json_object_get_string_member_with_default (m, "fit", "");

  if (json_object_get_boolean_member_with_default (m, "installed", FALSE))
    {
      *css_class = "success";
      return _("installed");
    }
  *css_class = "dim-label";
  if (g_strcmp0 (fit, "runs") == 0)
    return _("runs on this machine");
  if (g_strcmp0 (fit, "tight") == 0)
    return _("tight fit");
  if (g_strcmp0 (fit, "toobig") == 0)
    return _("too big — use a provider");
  return _("unknown fit");
}

static gboolean
model_can_get (JsonObject *m)
{
  const gchar *fit = json_object_get_string_member_with_default (m, "fit", "");

  return json_object_get_boolean_member_with_default (m, "available", FALSE) &&
         !json_object_get_boolean_member_with_default (m, "installed", FALSE) &&
         g_strcmp0 (fit, "toobig") != 0;
}

static void
on_get_finished (GObject *source, GAsyncResult *res, gpointer data)
{
  GSubprocess *proc = G_SUBPROCESS (source);
  g_autoptr (GError) error = NULL;

  if (!g_subprocess_wait_check_finish (proc, res, &error) &&
      g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
    return; /* panel gone — data may be invalid, touch nothing */

  refresh_models (CC_LISA_PANEL (data));
}

static void
on_get_clicked (GtkButton *button, gpointer data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  const gchar *id = g_object_get_data (G_OBJECT (button), "model-id");
  g_autoptr (GError) error = NULL;
  g_autoptr (GSubprocess) proc = NULL;

  if (!id)
    return;

  gtk_widget_set_sensitive (GTK_WIDGET (button), FALSE);
  gtk_button_set_label (button, _("Downloading…"));

  proc = g_subprocess_new (G_SUBPROCESS_FLAGS_STDOUT_SILENCE |
                             G_SUBPROCESS_FLAGS_STDERR_SILENCE,
                           &error, "lisa", "models", "get", id, NULL);
  if (!proc)
    {
      gtk_widget_set_sensitive (GTK_WIDGET (button), TRUE);
      gtk_button_set_label (button, _("Get"));
      return;
    }
  g_subprocess_wait_check_async (proc, self->cancellable, on_get_finished, self);
}

static void
add_model_row (CcLisaPanel *self, JsonObject *m)
{
  const gchar *id = json_object_get_string_member_with_default (m, "id", NULL);
  const gchar *css = NULL;
  const gchar *badge_text;
  g_autofree gchar *subtitle = NULL;
  GtkWidget *row, *badge;

  if (!id)
    return;

  row = adw_action_row_new ();
  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row), id);
  subtitle = model_subtitle (m);
  adw_action_row_set_subtitle (ADW_ACTION_ROW (row), subtitle);

  badge_text = model_badge (m, &css);
  badge = gtk_label_new (badge_text);
  gtk_widget_set_valign (badge, GTK_ALIGN_CENTER);
  gtk_widget_add_css_class (badge, "caption");
  gtk_widget_add_css_class (badge, css);
  adw_action_row_add_suffix (ADW_ACTION_ROW (row), badge);

  if (model_can_get (m))
    {
      GtkWidget *get = gtk_button_new_with_label (_("Get"));
      gtk_widget_set_valign (get, GTK_ALIGN_CENTER);
      gtk_widget_add_css_class (get, "suggested-action");
      gtk_widget_set_tooltip_text (get, _("Download this model to run it locally"));
      g_object_set_data_full (G_OBJECT (get), "model-id", g_strdup (id), g_free);
      g_signal_connect (get, "clicked", G_CALLBACK (on_get_clicked), self);
      adw_action_row_add_suffix (ADW_ACTION_ROW (row), get);
    }

  adw_preferences_group_add (self->models_group, row);
}

static void
on_catalog_ready (GObject *source, GAsyncResult *res, gpointer data)
{
  GSubprocess *proc = G_SUBPROCESS (source);
  g_autofree gchar *stdout_buf = NULL;
  g_autoptr (GError) error = NULL;
  g_autoptr (JsonParser) parser = NULL;
  CcLisaPanel *self;
  JsonObject *root_obj;
  JsonArray *models;

  if (!g_subprocess_communicate_utf8_finish (proc, res, &stdout_buf, NULL, &error))
    {
      if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
        return;
      /* fall through with empty stdout → the "couldn't read" message */
    }

  self = CC_LISA_PANEL (data);

  if (!g_subprocess_get_successful (proc) || !stdout_buf || !*stdout_buf)
    {
      adw_preferences_group_set_description (
        self->models_group,
        _("Could not read the local catalog (lisa models catalog --json). "
          "Is the lisa CLI on PATH and up to date?"));
      return;
    }

  parser = json_parser_new ();
  if (!json_parser_load_from_data (parser, stdout_buf, -1, &error))
    {
      adw_preferences_group_set_description (
        self->models_group, _("The model catalog could not be parsed."));
      return;
    }

  root_obj = json_node_get_object (json_parser_get_root (parser));
  if (root_obj && json_object_has_member (root_obj, "profile"))
    {
      JsonObject *p = json_object_get_object_member (root_obj, "profile");
      gint64 ram = json_object_get_int_member_with_default (p, "total_ram_gb", 0);
      gint64 tier = json_object_get_int_member_with_default (p, "tier", 0);
      g_autofree gchar *desc = g_strdup_printf (
        _("This machine: %" G_GINT64_FORMAT " GiB RAM · tier %" G_GINT64_FORMAT ". "
          "Local inference never leaves this machine."), ram, tier);
      adw_preferences_group_set_description (self->models_group, desc);
    }

  models = (root_obj && json_object_has_member (root_obj, "models"))
             ? json_object_get_array_member (root_obj, "models")
             : NULL;
  for (guint i = 0; models && i < json_array_get_length (models); i++)
    add_model_row (self, json_array_get_object_element (models, i));
}

static void
refresh_models (CcLisaPanel *self)
{
  g_autoptr (GError) error = NULL;
  g_autoptr (GSubprocess) proc = NULL;

  if (self->models_group)
    adw_preferences_page_remove (self->page, self->models_group);

  self->models_group = ADW_PREFERENCES_GROUP (adw_preferences_group_new ());
  adw_preferences_group_set_title (self->models_group, _("Local models"));
  adw_preferences_group_set_description (self->models_group, _("Reading catalog…"));
  adw_preferences_page_add (self->page, self->models_group);

  proc = g_subprocess_new (G_SUBPROCESS_FLAGS_STDOUT_PIPE |
                             G_SUBPROCESS_FLAGS_STDERR_SILENCE,
                           &error, "lisa", "models", "catalog", "--json", NULL);
  if (!proc)
    {
      adw_preferences_group_set_description (
        self->models_group,
        _("The lisa CLI was not found on PATH."));
      return;
    }
  g_subprocess_communicate_utf8_async (proc, NULL, self->cancellable,
                                       on_catalog_ready, self);
}

/* ------------------------------------------------------------------ */
/* Providers & privacy (bridge to org.lisa.Settings for v1)            */
/* ------------------------------------------------------------------ */

static void
on_manage_providers (GtkButton *button, gpointer data)
{
  g_autoptr (GDesktopAppInfo) info = g_desktop_app_info_new ("org.lisa.Settings.desktop");

  if (info)
    g_app_info_launch (G_APP_INFO (info), NULL, NULL, NULL);
}

static AdwPreferencesGroup *
build_providers_group (CcLisaPanel *self)
{
  GtkWidget *group = adw_preferences_group_new ();
  GtkWidget *row = adw_action_row_new ();
  GtkWidget *open = gtk_button_new_with_label (_("Open…"));

  adw_preferences_group_set_title (ADW_PREFERENCES_GROUP (group),
                                   _("Providers & privacy"));
  adw_preferences_group_set_description (
    ADW_PREFERENCES_GROUP (group),
    _("Bring-your-own model providers, API keys, and what may leave this "
      "machine. Requests through a provider leave your hardware and are "
      "marked in the Ledger."));

  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row),
                                 _("Manage providers and offload consent"));
  adw_action_row_set_subtitle (ADW_ACTION_ROW (row),
                               _("Opens Lisa AI settings"));
  gtk_widget_set_valign (open, GTK_ALIGN_CENTER);
  g_signal_connect (open, "clicked", G_CALLBACK (on_manage_providers), self);
  adw_action_row_add_suffix (ADW_ACTION_ROW (row), open);
  adw_action_row_set_activatable_widget (ADW_ACTION_ROW (row), open);
  adw_preferences_group_add (ADW_PREFERENCES_GROUP (group), row);

  return ADW_PREFERENCES_GROUP (group);
}

/* ------------------------------------------------------------------ */
/* GObject                                                             */
/* ------------------------------------------------------------------ */

static void
cc_lisa_panel_dispose (GObject *object)
{
  CcLisaPanel *self = CC_LISA_PANEL (object);

  g_cancellable_cancel (self->cancellable);
  g_clear_object (&self->cancellable);

  G_OBJECT_CLASS (cc_lisa_panel_parent_class)->dispose (object);
}

static void
cc_lisa_panel_class_init (CcLisaPanelClass *klass)
{
  GObjectClass *object_class = G_OBJECT_CLASS (klass);

  object_class->dispose = cc_lisa_panel_dispose;
}

static void
cc_lisa_panel_init (CcLisaPanel *self)
{
  GtkWidget *toolbar_view = adw_toolbar_view_new ();
  GtkWidget *header = adw_header_bar_new ();

  self->cancellable = g_cancellable_new ();
  self->page = ADW_PREFERENCES_PAGE (adw_preferences_page_new ());

  adw_preferences_page_add (self->page, build_providers_group (self));
  refresh_models (self); /* adds the models group + loads it async */

  adw_toolbar_view_add_top_bar (ADW_TOOLBAR_VIEW (toolbar_view), header);
  adw_toolbar_view_set_content (ADW_TOOLBAR_VIEW (toolbar_view),
                                GTK_WIDGET (self->page));

  adw_navigation_page_set_title (ADW_NAVIGATION_PAGE (self), _("Intelligence"));
  adw_navigation_page_set_child (ADW_NAVIGATION_PAGE (self), toolbar_view);
}
