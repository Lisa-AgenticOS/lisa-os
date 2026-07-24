/* cc-lisa-panel.c — Lisa "Intelligence" panel for GNOME Settings.
 *
 * PLAN §5.3 (Settings panel), §5.11, §8; ADR-0008, ADR-0012.
 *
 * A root menu of two rows (System-style list-in-list) over an
 * AdwNavigationView, each pushing a subpage:
 *   - Providers: bring-your-own OpenAI-compatible endpoints and their
 *     write-only API keys, read live from the lisa-remoted broker over the
 *     session bus (org.lisa.Remote1). Add/remove providers, set or clear a
 *     key, and — for OAuth-capable providers (Anthropic, OpenAI) — "Sign in
 *     with Claude"/"Sign in with ChatGPT" via BeginLogin (the broker runs
 *     the PKCE flow; we open the browser and react to LoginCompleted). Also
 *     the per-scope "may offload" consent switch: nothing leaves this
 *     machine until a scope is switched on; remote models need the Prompts
 *     scope. Everything here can cause egress and is marked in the Ledger.
 *   - Local models: `lisa models catalog --json` (§8 hardware-aware fit),
 *     each row badged by what runs on THIS machine, with a one-click Get
 *     (`lisa models get`). Local inference never leaves the machine.
 *
 * Providers + consent used to live in a standalone org.lisa.Settings app
 * behind an "Open…" button; they are now native here and that app is
 * hidden (NoDisplay). If the broker is not running, the providers group
 * shows a single inline "broker not running" row and disables the write
 * actions — the panel never crashes.
 *
 * Programmatic UI (no .ui/gresource): CcPanel derives AdwNavigationPage,
 * so we set an AdwToolbarView + AdwToastOverlay + AdwPreferencesPage as
 * its child.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include <adwaita.h>
#include <glib/gi18n.h>
#include <json-glib/json-glib.h>

#include "cc-lisa-panel.h"

/* The broker's management surface (ADR-0008 §1). Session bus. */
#define REMOTE_BUS_NAME    "org.lisa.Remoted"
#define REMOTE_OBJECT_PATH "/org/lisa/Remote1"
#define REMOTE_IFACE       "org.lisa.Remote1"
#define REMOTE_TIMEOUT_MS  3000

struct _CcLisaPanel
{
  CcPanel parent_instance;

  AdwNavigationView   *nav;             /* root menu → Providers / Local models */
  AdwPreferencesPage  *page;            /* Providers subpage: providers+consent */
  AdwPreferencesPage  *models_page;     /* Local models subpage                 */
  AdwToastOverlay     *toasts;

  AdwPreferencesGroup *providers_group; /* stable; rows rebuilt from State */
  AdwPreferencesGroup *consent_group;   /* stable; rows rebuilt from State */
  AdwPreferencesGroup *models_group;    /* rebuilt on every model refresh  */
  GtkWidget           *add_provider_btn;

  GPtrArray           *provider_rows;   /* borrowed row widgets            */
  GPtrArray           *consent_rows;    /* borrowed row widgets            */
  GPtrArray           *known_ids;       /* owned char* ids, for validation */

  GDBusProxy          *remote;          /* NULL until resolved / if absent */
  GCancellable        *cancellable;
};

CC_PANEL_REGISTER (CcLisaPanel, cc_lisa_panel)

/* The offloadable scopes, mirroring the broker's consent table. The
 * `prompt` scope is first and load-bearing: inferenced always sends it,
 * so a remote request is refused while it is off. */
typedef struct
{
  const gchar *id;
  const gchar *label;
  const gchar *description;
} LisaScope;

static const LisaScope SCOPES[] = {
  { "prompt",   N_("Prompts"),    N_("The text you type into assistant requests") },
  { "files",    N_("Files"),      N_("Document chunks retrieved from your files") },
  { "mail",     N_("Mail"),       N_("Mail content retrieved as context") },
  { "calendar", N_("Calendar"),   N_("Calendar and contact context") },
  { "screen",   N_("Screen"),     N_("Screen captures you attach to a request") },
  { "memory",   N_("App memory"), N_("Per-app durable memory contents") },
};

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
    adw_preferences_page_remove (self->models_page, self->models_group);

  self->models_group = ADW_PREFERENCES_GROUP (adw_preferences_group_new ());
  adw_preferences_group_set_title (self->models_group, _("Local models"));
  adw_preferences_group_set_description (self->models_group, _("Reading catalog…"));
  adw_preferences_page_add (self->models_page, self->models_group);

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
/* Cloud providers + Privacy & offload (native, over org.lisa.Remote1) */
/* ------------------------------------------------------------------ */

/* A modal dialog's context: entries + provider id, freed with the
 * dialog. Unused fields stay NULL for whichever dialog owns it. */
typedef struct
{
  CcLisaPanel *self;
  AdwDialog   *dialog;
  GtkEditable *entry_key;   /* key dialog */
  GtkEditable *entry_id;    /* add-provider dialog */
  GtkEditable *entry_name;
  GtkEditable *entry_url;
  gchar       *provider_id; /* key dialog target */
} DialogCtx;

static void refresh_state (CcLisaPanel *self);
static void rebuild_providers (CcLisaPanel *self, JsonObject *state, gboolean offline);
static void rebuild_consent (CcLisaPanel *self, JsonObject *state, gboolean offline);

static void
lisa_toast (CcLisaPanel *self, const gchar *message)
{
  if (self->toasts && message && *message)
    adw_toast_overlay_add_toast (self->toasts, adw_toast_new (message));
}

static void
dialog_ctx_free (gpointer data)
{
  DialogCtx *ctx = data;

  g_free (ctx->provider_id);
  g_free (ctx);
}

/* Drop every tracked row from a group and empty the tracking array. The
 * group holds the only strong ref, so removal destroys the widget. */
static void
group_clear (AdwPreferencesGroup *group, GPtrArray *rows)
{
  for (guint i = 0; i < rows->len; i++)
    adw_preferences_group_remove (group, GTK_WIDGET (g_ptr_array_index (rows, i)));
  g_ptr_array_set_size (rows, 0);
}

static gint
cmp_provider_id (gconstpointer a, gconstpointer b)
{
  JsonObject *pa = *(JsonObject * const *) a;
  JsonObject *pb = *(JsonObject * const *) b;

  return g_strcmp0 (json_object_get_string_member_with_default (pa, "id", ""),
                    json_object_get_string_member_with_default (pb, "id", ""));
}

/* id ~ /^[a-z0-9][a-z0-9_-]*$/ (mirrors validateCustomProvider). */
static gboolean
valid_provider_id (const gchar *id)
{
  if (!id || !*id)
    return FALSE;
  for (const gchar *p = id; *p; p++)
    {
      gchar c = *p;
      gboolean base = (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9');
      if (p == id)
        {
          if (!base)
            return FALSE;
        }
      else if (!(base || c == '-' || c == '_'))
        return FALSE;
    }
  return TRUE;
}

/* Completion for a mutating broker call: on failure toast the reason,
 * then re-read authoritative State either way. Cancelled → panel gone. */
static void
on_mutate_done (GObject *source, GAsyncResult *res, gpointer data)
{
  g_autoptr (GError) error = NULL;
  g_autoptr (GVariant) reply =
    g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);

  if (!reply)
    {
      if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
        return;
      lisa_toast (CC_LISA_PANEL (data), error->message);
      refresh_state (CC_LISA_PANEL (data));
      return;
    }
  refresh_state (CC_LISA_PANEL (data));
}

static void
on_state_ready (GObject *source, GAsyncResult *res, gpointer data)
{
  g_autoptr (GError) error = NULL;
  g_autoptr (GVariant) reply =
    g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);
  g_autoptr (JsonParser) parser = NULL;
  CcLisaPanel *self;
  const gchar *json = NULL;
  JsonObject *root = NULL;
  JsonNode *node;

  if (!reply)
    {
      if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
        return;
      self = CC_LISA_PANEL (data);
      rebuild_providers (self, NULL, TRUE);
      rebuild_consent (self, NULL, TRUE);
      return;
    }

  self = CC_LISA_PANEL (data);
  g_variant_get (reply, "(&s)", &json);

  parser = json_parser_new ();
  if (json && *json && json_parser_load_from_data (parser, json, -1, NULL))
    {
      node = json_parser_get_root (parser);
      if (node && JSON_NODE_HOLDS_OBJECT (node))
        root = json_node_get_object (node);
    }

  rebuild_providers (self, root, FALSE);
  rebuild_consent (self, root, FALSE);
}

/* Read State() when the broker owns its name; otherwise render the
 * broker-absent fallback without auto-starting anything. */
static void
refresh_state (CcLisaPanel *self)
{
  gboolean reachable = self->remote != NULL;

  if (reachable)
    {
      g_autofree gchar *owner = g_dbus_proxy_get_name_owner (self->remote);
      reachable = owner != NULL;
    }

  if (!reachable)
    {
      rebuild_providers (self, NULL, TRUE);
      rebuild_consent (self, NULL, TRUE);
      return;
    }

  g_dbus_proxy_call (self->remote, "State", NULL, G_DBUS_CALL_FLAGS_NONE,
                     REMOTE_TIMEOUT_MS, self->cancellable, on_state_ready, self);
}

/* --- Privacy & offload ------------------------------------------------ */

static void
on_consent_toggled (GObject *row, GParamSpec *pspec, gpointer data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  const gchar *scope = g_object_get_data (row, "scope");
  gboolean active = adw_switch_row_get_active (ADW_SWITCH_ROW (row));

  if (!self->remote || !scope)
    return;

  g_dbus_proxy_call (self->remote, "SetConsent",
                     g_variant_new ("(sb)", scope, active),
                     G_DBUS_CALL_FLAGS_NONE, REMOTE_TIMEOUT_MS,
                     self->cancellable, on_mutate_done, self);
}

static void
rebuild_consent (CcLisaPanel *self, JsonObject *state, gboolean offline)
{
  JsonObject *may_offload = NULL;

  group_clear (self->consent_group, self->consent_rows);

  if (!offline && state && json_object_has_member (state, "may_offload"))
    may_offload = json_object_get_object_member (state, "may_offload");

  for (guint i = 0; i < G_N_ELEMENTS (SCOPES); i++)
    {
      const LisaScope *s = &SCOPES[i];
      gboolean active = may_offload
        ? json_object_get_boolean_member_with_default (may_offload, s->id, FALSE)
        : FALSE;
      GtkWidget *row = adw_switch_row_new ();
      g_autofree gchar *subtitle = NULL;
      GtkWidget *cue;

      adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row), _(s->label));
      if (g_strcmp0 (s->id, "prompt") == 0)
        subtitle = g_strdup_printf (_("%s — required for any remote request"),
                                    _(s->description));
      else
        subtitle = g_strdup (_(s->description));
      adw_action_row_set_subtitle (ADW_ACTION_ROW (row), subtitle);

      /* Set active BEFORE connecting so the programmatic value never
       * fires our handler (which would echo a needless SetConsent). */
      adw_switch_row_set_active (ADW_SWITCH_ROW (row), active);
      gtk_widget_set_sensitive (row, !offline);

      /* The egress cue: a switched-on scope permits data to leave the
       * machine — say so, in the amber "warning" style when live. */
      cue = gtk_label_new (_("leaves your hardware"));
      gtk_widget_set_valign (cue, GTK_ALIGN_CENTER);
      gtk_widget_add_css_class (cue, "caption");
      gtk_widget_add_css_class (cue, active ? "warning" : "dim-label");
      adw_action_row_add_suffix (ADW_ACTION_ROW (row), cue);

      g_object_set_data_full (G_OBJECT (row), "scope", g_strdup (s->id), g_free);
      g_signal_connect (row, "notify::active",
                        G_CALLBACK (on_consent_toggled), self);

      adw_preferences_group_add (self->consent_group, row);
      g_ptr_array_add (self->consent_rows, row);
    }
}

/* --- Cloud providers -------------------------------------------------- */

static void
on_clear_clicked (GtkButton *button, gpointer data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  const gchar *id = g_object_get_data (G_OBJECT (button), "pid");

  if (!self->remote || !id)
    return;

  g_dbus_proxy_call (self->remote, "ClearKey", g_variant_new ("(s)", id),
                     G_DBUS_CALL_FLAGS_NONE, REMOTE_TIMEOUT_MS,
                     self->cancellable, on_mutate_done, self);
}

static void
on_remove_clicked (GtkButton *button, gpointer data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  const gchar *id = g_object_get_data (G_OBJECT (button), "pid");

  if (!self->remote || !id)
    return;

  g_dbus_proxy_call (self->remote, "RemoveProvider", g_variant_new ("(s)", id),
                     G_DBUS_CALL_FLAGS_NONE, REMOTE_TIMEOUT_MS,
                     self->cancellable, on_mutate_done, self);
}

static void
on_key_save (GtkButton *button, gpointer data)
{
  DialogCtx *ctx = data;
  CcLisaPanel *self = ctx->self;
  g_autofree gchar *key = g_strdup (gtk_editable_get_text (ctx->entry_key));

  g_strstrip (key);
  if (!*key)
    {
      lisa_toast (self, _("Key must not be empty."));
      return;
    }
  if (self->remote)
    g_dbus_proxy_call (self->remote, "SetKey",
                       g_variant_new ("(ss)", ctx->provider_id, key),
                       G_DBUS_CALL_FLAGS_NONE, REMOTE_TIMEOUT_MS,
                       self->cancellable, on_mutate_done, self);
  adw_dialog_close (ctx->dialog);
}

/* Write-only key entry: store or replace a credential. The broker never
 * hands key material back, so there is nothing to prefill. */
static void
on_key_clicked (GtkButton *button, gpointer data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  const gchar *id = g_object_get_data (G_OBJECT (button), "pid");
  AdwDialog *dialog;
  GtkWidget *page, *group, *entry, *toolbar, *header, *cancel, *save;
  DialogCtx *ctx;

  if (!id)
    return;

  dialog = ADW_DIALOG (adw_dialog_new ());
  adw_dialog_set_title (dialog, _("Provider key"));
  adw_dialog_set_content_width (dialog, 460);

  page = adw_preferences_page_new ();
  group = adw_preferences_group_new ();
  adw_preferences_group_set_description (
    ADW_PREFERENCES_GROUP (group),
    _("Stored write-only by the broker — it can be replaced or forgotten, "
      "never read back."));
  entry = adw_password_entry_row_new ();
  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (entry), _("API key"));
  adw_preferences_group_add (ADW_PREFERENCES_GROUP (group), entry);
  adw_preferences_page_add (ADW_PREFERENCES_PAGE (page), ADW_PREFERENCES_GROUP (group));

  toolbar = adw_toolbar_view_new ();
  header = adw_header_bar_new ();
  adw_header_bar_set_show_end_title_buttons (ADW_HEADER_BAR (header), FALSE);
  cancel = gtk_button_new_with_label (_("Cancel"));
  save = gtk_button_new_with_label (_("Save"));
  gtk_widget_add_css_class (save, "suggested-action");
  adw_header_bar_pack_start (ADW_HEADER_BAR (header), cancel);
  adw_header_bar_pack_end (ADW_HEADER_BAR (header), save);
  adw_toolbar_view_add_top_bar (ADW_TOOLBAR_VIEW (toolbar), header);
  adw_toolbar_view_set_content (ADW_TOOLBAR_VIEW (toolbar), page);
  adw_dialog_set_child (dialog, toolbar);

  ctx = g_new0 (DialogCtx, 1);
  ctx->self = self;
  ctx->dialog = dialog;
  ctx->entry_key = GTK_EDITABLE (entry);
  ctx->provider_id = g_strdup (id);
  g_object_set_data_full (G_OBJECT (dialog), "ctx", ctx, dialog_ctx_free);

  g_signal_connect_swapped (cancel, "clicked",
                            G_CALLBACK (adw_dialog_close), dialog);
  g_signal_connect (save, "clicked", G_CALLBACK (on_key_save), ctx);

  adw_dialog_present (dialog, GTK_WIDGET (self));
}

/* --- OAuth sign-in (Anthropic/OpenAI) --------------------------------- */

/* The broker owns the flow (it is the only daemon with egress): BeginLogin
 * starts a PKCE + localhost-callback exchange and returns the authorize
 * URL, which we open in the user's browser. The broker emits LoginCompleted
 * when the exchange lands, which drives a toast + State() re-read. */

static const gchar *
provider_signin_verb (const gchar *id)
{
  if (g_strcmp0 (id, "anthropic") == 0)
    return _("Sign in with Claude");
  if (g_strcmp0 (id, "openai") == 0)
    return _("Sign in with ChatGPT");
  return _("Sign in");
}

static void
on_beginlogin_done (GObject *source, GAsyncResult *res, gpointer data)
{
  g_autoptr (GError) error = NULL;
  g_autoptr (GVariant) reply =
    g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);
  const gchar *url = NULL;

  if (!reply)
    {
      if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
        return;
      lisa_toast (CC_LISA_PANEL (data), error->message);
      return;
    }

  g_variant_get (reply, "(&s)", &url);
  if (url && *url)
    g_app_info_launch_default_for_uri (url, NULL, NULL);
}

static void
on_signin_clicked (GtkButton *button, gpointer data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  const gchar *id = g_object_get_data (G_OBJECT (button), "pid");

  if (!self->remote || !id)
    return;

  lisa_toast (self, _("Opening your browser to sign in…"));
  g_dbus_proxy_call (self->remote, "BeginLogin", g_variant_new ("(s)", id),
                     G_DBUS_CALL_FLAGS_NONE, REMOTE_TIMEOUT_MS,
                     self->cancellable, on_beginlogin_done, self);
}

static void
on_logout_clicked (GtkButton *button, gpointer data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  const gchar *id = g_object_get_data (G_OBJECT (button), "pid");

  if (!self->remote || !id)
    return;

  g_dbus_proxy_call (self->remote, "Logout", g_variant_new ("(s)", id),
                     G_DBUS_CALL_FLAGS_NONE, REMOTE_TIMEOUT_MS,
                     self->cancellable, on_mutate_done, self);
}

/* The broker's LoginCompleted(provider, ok, detail): toast the outcome and
 * re-read State so the row flips to "Signed in". */
static void
on_remote_signal (GDBusProxy  *proxy,
                  const gchar *sender,
                  const gchar *signal,
                  GVariant    *params,
                  gpointer     data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  const gchar *pid = NULL, *detail = NULL;
  gboolean ok = FALSE;

  if (g_strcmp0 (signal, "LoginCompleted") != 0)
    return;

  g_variant_get (params, "(&sb&s)", &pid, &ok, &detail);
  lisa_toast (self, (detail && *detail) ? detail
                      : (ok ? _("Signed in.") : _("Sign-in failed.")));
  refresh_state (self);
}

static void
add_provider_row (CcLisaPanel *self, JsonObject *p)
{
  const gchar *id = json_object_get_string_member_with_default (p, "id", NULL);
  const gchar *name;
  const gchar *base;
  gboolean has_cred, builtin, oauth_capable, connected;
  g_autoptr (GString) subtitle = NULL;
  GtkWidget *row, *pill, *keybtn;

  if (!id)
    return;

  name = json_object_get_string_member_with_default (p, "display_name", id);
  base = json_object_get_string_member_with_default (p, "base_url", NULL);
  /* has_key = a stored API key specifically (not an OAuth sign-in), so the
   * key controls stay honest for a provider that is only signed in. */
  has_cred = json_object_get_boolean_member_with_default (p, "has_key", FALSE);
  builtin = json_object_get_boolean_member_with_default (p, "builtin", FALSE);
  /* Broker (ADR-0010) marks providers that support OAuth sign-in and whether
   * a usable token is stored. Defaults keep older brokers working. */
  oauth_capable = json_object_get_boolean_member_with_default (p, "oauth_capable", FALSE);
  connected = json_object_get_boolean_member_with_default (p, "connected", FALSE);

  subtitle = g_string_new ((base && *base) ? base : _("endpoint not configured"));
  if (oauth_capable)
    g_string_append_printf (subtitle, " · %s",
                            connected ? _("signed in") : _("not signed in"));
  g_string_append_printf (subtitle, " · %s", has_cred ? _("key set") : _("no key"));
  if (!builtin)
    g_string_append_printf (subtitle, " · %s", _("custom"));

  row = adw_action_row_new ();
  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row), name);
  adw_action_row_set_subtitle (ADW_ACTION_ROW (row), subtitle->str);

  {
    /* An OAuth provider's headline status is its sign-in, not the key. */
    gboolean pill_ok = oauth_capable ? connected : has_cred;
    const gchar *pill_text = oauth_capable
      ? (connected ? _("Signed in") : _("Not signed in"))
      : (has_cred ? _("Key set") : _("No key"));
    pill = gtk_label_new (pill_text);
    gtk_widget_set_valign (pill, GTK_ALIGN_CENTER);
    gtk_widget_add_css_class (pill, "caption");
    gtk_widget_add_css_class (pill, pill_ok ? "success" : "dim-label");
    adw_action_row_add_suffix (ADW_ACTION_ROW (row), pill);
  }

  /* Sign in with Claude / ChatGPT — for oauth-capable providers, alongside
   * the manual API key below. */
  if (oauth_capable)
    {
      GtkWidget *auth = gtk_button_new_with_label (
        connected ? _("Sign out") : provider_signin_verb (id));
      gtk_widget_set_valign (auth, GTK_ALIGN_CENTER);
      if (!connected)
        gtk_widget_add_css_class (auth, "suggested-action");
      g_object_set_data_full (G_OBJECT (auth), "pid", g_strdup (id), g_free);
      g_signal_connect (auth, "clicked",
                        G_CALLBACK (connected ? on_logout_clicked
                                              : on_signin_clicked),
                        self);
      adw_action_row_add_suffix (ADW_ACTION_ROW (row), auth);
    }

  keybtn = gtk_button_new_with_label (has_cred ? _("Replace key…") : _("Set key…"));
  gtk_widget_set_valign (keybtn, GTK_ALIGN_CENTER);
  g_object_set_data_full (G_OBJECT (keybtn), "pid", g_strdup (id), g_free);
  g_signal_connect (keybtn, "clicked", G_CALLBACK (on_key_clicked), self);
  adw_action_row_add_suffix (ADW_ACTION_ROW (row), keybtn);

  if (has_cred)
    {
      GtkWidget *clear = gtk_button_new_from_icon_name ("edit-clear-symbolic");
      gtk_widget_set_valign (clear, GTK_ALIGN_CENTER);
      gtk_widget_set_tooltip_text (clear, _("Forget the stored key"));
      g_object_set_data_full (G_OBJECT (clear), "pid", g_strdup (id), g_free);
      g_signal_connect (clear, "clicked", G_CALLBACK (on_clear_clicked), self);
      adw_action_row_add_suffix (ADW_ACTION_ROW (row), clear);
    }

  if (!builtin)
    {
      GtkWidget *remove = gtk_button_new_from_icon_name ("user-trash-symbolic");
      gtk_widget_set_valign (remove, GTK_ALIGN_CENTER);
      gtk_widget_set_tooltip_text (remove, _("Remove this provider and its key"));
      g_object_set_data_full (G_OBJECT (remove), "pid", g_strdup (id), g_free);
      g_signal_connect (remove, "clicked", G_CALLBACK (on_remove_clicked), self);
      adw_action_row_add_suffix (ADW_ACTION_ROW (row), remove);
    }

  adw_preferences_group_add (self->providers_group, row);
  g_ptr_array_add (self->provider_rows, row);
  g_ptr_array_add (self->known_ids, g_strdup (id));
}

static void
rebuild_providers (CcLisaPanel *self, JsonObject *state, gboolean offline)
{
  JsonArray *providers;
  guint n;
  g_autoptr (GPtrArray) ordered = NULL; /* borrowed JsonObject* */
  g_autoptr (GPtrArray) custom = NULL;

  group_clear (self->providers_group, self->provider_rows);
  g_ptr_array_set_size (self->known_ids, 0);
  gtk_widget_set_sensitive (self->add_provider_btn, !offline);

  if (offline)
    {
      GtkWidget *row = adw_action_row_new ();
      adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row),
                                     _("Provider broker not running"));
      adw_action_row_set_subtitle (
        ADW_ACTION_ROW (row),
        _("Start lisa-remoted to add cloud providers and set what may leave "
          "this machine."));
      adw_preferences_group_add (self->providers_group, row);
      g_ptr_array_add (self->provider_rows, row);
      return;
    }

  providers = (state && json_object_has_member (state, "providers"))
                ? json_object_get_array_member (state, "providers")
                : NULL;
  n = providers ? json_array_get_length (providers) : 0;

  /* Built-ins first in registry order, then custom endpoints by id. */
  ordered = g_ptr_array_new ();
  custom = g_ptr_array_new ();
  for (guint i = 0; i < n; i++)
    {
      JsonObject *p = json_array_get_object_element (providers, i);
      if (json_object_get_boolean_member_with_default (p, "builtin", FALSE))
        g_ptr_array_add (ordered, p);
      else
        g_ptr_array_add (custom, p);
    }
  g_ptr_array_sort (custom, cmp_provider_id);
  for (guint i = 0; i < custom->len; i++)
    g_ptr_array_add (ordered, g_ptr_array_index (custom, i));

  for (guint i = 0; i < ordered->len; i++)
    add_provider_row (self, g_ptr_array_index (ordered, i));
}

/* --- Add custom provider --------------------------------------------- */

static void
on_add_save (GtkButton *button, gpointer data)
{
  DialogCtx *ctx = data;
  CcLisaPanel *self = ctx->self;
  g_autofree gchar *id = g_strdup (gtk_editable_get_text (ctx->entry_id));
  g_autofree gchar *name = g_strdup (gtk_editable_get_text (ctx->entry_name));
  g_autofree gchar *url = g_strdup (gtk_editable_get_text (ctx->entry_url));
  g_autoptr (GString) err = g_string_new (NULL);

  g_strstrip (id);
  g_strstrip (name);
  g_strstrip (url);

  if (!valid_provider_id (id))
    {
      g_string_append (err, _("Id must be lowercase letters, digits, ‘-’ or ‘_’. "));
    }
  else
    {
      for (guint i = 0; i < self->known_ids->len; i++)
        if (g_strcmp0 (id, g_ptr_array_index (self->known_ids, i)) == 0)
          {
            g_string_append_printf (err, _("Id “%s” is already taken. "), id);
            break;
          }
    }
  if (!*name)
    g_string_append (err, _("Name must not be empty. "));
  if (!(g_str_has_prefix (url, "https://") || g_str_has_prefix (url, "http://")))
    g_string_append (err,
                     _("Base URL must start with https:// (or http:// for local endpoints). "));

  if (err->len > 0)
    {
      lisa_toast (self, err->str);
      return;
    }

  if (self->remote)
    g_dbus_proxy_call (self->remote, "AddProvider",
                       g_variant_new ("(sss)", id, name, url),
                       G_DBUS_CALL_FLAGS_NONE, REMOTE_TIMEOUT_MS,
                       self->cancellable, on_mutate_done, self);
  adw_dialog_close (ctx->dialog);
}

static void
on_add_clicked (GtkButton *button, gpointer data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  AdwDialog *dialog;
  GtkWidget *page, *group, *id_row, *name_row, *url_row;
  GtkWidget *toolbar, *header, *cancel, *add;
  DialogCtx *ctx;

  dialog = ADW_DIALOG (adw_dialog_new ());
  adw_dialog_set_title (dialog, _("Add provider"));
  adw_dialog_set_content_width (dialog, 460);

  page = adw_preferences_page_new ();
  group = adw_preferences_group_new ();
  adw_preferences_group_set_description (
    ADW_PREFERENCES_GROUP (group),
    _("Any OpenAI-compatible endpoint — your own box, or a service you have "
      "an account with (§5.11)."));

  id_row = adw_entry_row_new ();
  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (id_row), _("Id (e.g. homelab)"));
  name_row = adw_entry_row_new ();
  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (name_row), _("Name"));
  url_row = adw_entry_row_new ();
  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (url_row),
                                 _("Base URL (OpenAI-compatible, …/v1)"));
  adw_preferences_group_add (ADW_PREFERENCES_GROUP (group), id_row);
  adw_preferences_group_add (ADW_PREFERENCES_GROUP (group), name_row);
  adw_preferences_group_add (ADW_PREFERENCES_GROUP (group), url_row);
  adw_preferences_page_add (ADW_PREFERENCES_PAGE (page), ADW_PREFERENCES_GROUP (group));

  toolbar = adw_toolbar_view_new ();
  header = adw_header_bar_new ();
  adw_header_bar_set_show_end_title_buttons (ADW_HEADER_BAR (header), FALSE);
  cancel = gtk_button_new_with_label (_("Cancel"));
  add = gtk_button_new_with_label (_("Add"));
  gtk_widget_add_css_class (add, "suggested-action");
  adw_header_bar_pack_start (ADW_HEADER_BAR (header), cancel);
  adw_header_bar_pack_end (ADW_HEADER_BAR (header), add);
  adw_toolbar_view_add_top_bar (ADW_TOOLBAR_VIEW (toolbar), header);
  adw_toolbar_view_set_content (ADW_TOOLBAR_VIEW (toolbar), page);
  adw_dialog_set_child (dialog, toolbar);

  ctx = g_new0 (DialogCtx, 1);
  ctx->self = self;
  ctx->dialog = dialog;
  ctx->entry_id = GTK_EDITABLE (id_row);
  ctx->entry_name = GTK_EDITABLE (name_row);
  ctx->entry_url = GTK_EDITABLE (url_row);
  g_object_set_data_full (G_OBJECT (dialog), "ctx", ctx, dialog_ctx_free);

  g_signal_connect_swapped (cancel, "clicked",
                            G_CALLBACK (adw_dialog_close), dialog);
  g_signal_connect (add, "clicked", G_CALLBACK (on_add_save), ctx);

  adw_dialog_present (dialog, GTK_WIDGET (self));
}

/* --- Broker connection ------------------------------------------------ */

static void
on_proxy_ready (GObject *source, GAsyncResult *res, gpointer data)
{
  g_autoptr (GError) error = NULL;
  GDBusProxy *proxy = g_dbus_proxy_new_for_bus_finish (res, &error);
  CcLisaPanel *self;

  if (!proxy)
    {
      /* Cancelled → the panel is gone, touch nothing. Otherwise the
       * broker-absent fallback (already shown) simply stays. */
      return;
    }

  self = CC_LISA_PANEL (data);
  self->remote = proxy;
  g_signal_connect (proxy, "g-signal", G_CALLBACK (on_remote_signal), self);
  refresh_state (self);
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
  g_clear_object (&self->remote);
  g_clear_pointer (&self->provider_rows, g_ptr_array_unref);
  g_clear_pointer (&self->consent_rows, g_ptr_array_unref);
  g_clear_pointer (&self->known_ids, g_ptr_array_unref);

  G_OBJECT_CLASS (cc_lisa_panel_parent_class)->dispose (object);
}

static void
cc_lisa_panel_class_init (CcLisaPanelClass *klass)
{
  GObjectClass *object_class = G_OBJECT_CLASS (klass);

  object_class->dispose = cc_lisa_panel_dispose;
}

/* Root-menu row → push its subpage (like the System panel's list-in-list). */
static void
on_menu_row_activated (AdwActionRow *row, gpointer data)
{
  CcLisaPanel *self = CC_LISA_PANEL (data);
  const gchar *tag = g_object_get_data (G_OBJECT (row), "nav-tag");

  if (self->nav && tag)
    adw_navigation_view_push_by_tag (self->nav, tag);
}

/* Build one root-menu row: title/subtitle, an icon, a chevron, activatable,
 * and tagged with the subpage it pushes. */
static GtkWidget *
menu_row_new (CcLisaPanel *self,
              const gchar *title,
              const gchar *subtitle,
              const gchar *icon,
              const gchar *tag)
{
  GtkWidget *row = adw_action_row_new ();

  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row), title);
  adw_action_row_set_subtitle (ADW_ACTION_ROW (row), subtitle);
  adw_action_row_add_prefix (ADW_ACTION_ROW (row),
                             gtk_image_new_from_icon_name (icon));
  adw_action_row_add_suffix (ADW_ACTION_ROW (row),
                             gtk_image_new_from_icon_name ("go-next-symbolic"));
  gtk_list_box_row_set_activatable (GTK_LIST_BOX_ROW (row), TRUE);
  g_object_set_data_full (G_OBJECT (row), "nav-tag", g_strdup (tag), g_free);
  g_signal_connect (row, "activated", G_CALLBACK (on_menu_row_activated), self);
  return row;
}

/* Wrap a preferences page in a titled, tagged AdwNavigationPage (its own
 * header bar; AdwNavigationView adds the back button on push). */
static AdwNavigationPage *
subpage_new (GtkWidget *content, const gchar *title, const gchar *tag)
{
  GtkWidget *toolbar = adw_toolbar_view_new ();
  AdwNavigationPage *page;

  adw_toolbar_view_add_top_bar (ADW_TOOLBAR_VIEW (toolbar), adw_header_bar_new ());
  adw_toolbar_view_set_content (ADW_TOOLBAR_VIEW (toolbar), content);
  page = adw_navigation_page_new (toolbar, title);
  adw_navigation_page_set_tag (page, tag);
  return page;
}

static void
cc_lisa_panel_init (CcLisaPanel *self)
{
  GtkWidget *root_page, *menu_group;
  AdwNavigationPage *root_nav, *providers_nav, *models_nav;

  self->cancellable = g_cancellable_new ();
  self->provider_rows = g_ptr_array_new ();
  self->consent_rows = g_ptr_array_new ();
  self->known_ids = g_ptr_array_new_with_free_func (g_free);
  self->page = ADW_PREFERENCES_PAGE (adw_preferences_page_new ());
  self->models_page = ADW_PREFERENCES_PAGE (adw_preferences_page_new ());
  self->toasts = ADW_TOAST_OVERLAY (adw_toast_overlay_new ());
  self->nav = ADW_NAVIGATION_VIEW (adw_navigation_view_new ());

  /* --- Providers subpage: Cloud providers + Privacy & offload -------- */
  self->providers_group = ADW_PREFERENCES_GROUP (adw_preferences_group_new ());
  adw_preferences_group_set_title (self->providers_group, _("Cloud providers"));
  adw_preferences_group_set_description (
    self->providers_group,
    _("Bring-your-own accounts and API keys. A request routed through a "
      "provider leaves your hardware and is marked in the Ledger. Keys are "
      "write-only — stored or cleared here, never read back."));
  self->add_provider_btn = gtk_button_new_from_icon_name ("list-add-symbolic");
  gtk_widget_set_valign (self->add_provider_btn, GTK_ALIGN_CENTER);
  gtk_widget_set_tooltip_text (self->add_provider_btn,
                               _("Add an OpenAI-compatible provider"));
  g_signal_connect (self->add_provider_btn, "clicked",
                    G_CALLBACK (on_add_clicked), self);
  adw_preferences_group_set_header_suffix (self->providers_group,
                                           self->add_provider_btn);
  adw_preferences_page_add (self->page, self->providers_group);

  self->consent_group = ADW_PREFERENCES_GROUP (adw_preferences_group_new ());
  adw_preferences_group_set_title (self->consent_group, _("Privacy & offload"));
  adw_preferences_group_set_description (
    self->consent_group,
    _("Nothing leaves this machine until you allow it, per scope. Remote "
      "models need the Prompts scope on; every switched-on scope permits "
      "egress and is marked in the Ledger."));
  adw_preferences_page_add (self->page, self->consent_group);

  /* Local models subpage — its group is (re)added on every refresh. */
  refresh_models (self);

  /* --- Root menu: two rows into the subpages (System-style) ---------- */
  menu_group = adw_preferences_group_new ();
  adw_preferences_group_add (
    ADW_PREFERENCES_GROUP (menu_group),
    menu_row_new (self, _("Providers"),
                  _("Cloud sign-in, API keys, and what may leave this machine"),
                  "network-transmit-receive-symbolic", "providers"));
  adw_preferences_group_add (
    ADW_PREFERENCES_GROUP (menu_group),
    menu_row_new (self, _("Local models"),
                  _("Models that run on this machine — never leave it"),
                  "computer-symbolic", "models"));
  root_page = adw_preferences_page_new ();
  adw_preferences_page_add (ADW_PREFERENCES_PAGE (root_page),
                            ADW_PREFERENCES_GROUP (menu_group));

  /* Root added FIRST so AdwNavigationView shows it; subpages are then
   * available for push_by_tag. */
  root_nav      = subpage_new (root_page, _("Intelligence"), "root");
  providers_nav = subpage_new (GTK_WIDGET (self->page), _("Providers"), "providers");
  models_nav    = subpage_new (GTK_WIDGET (self->models_page), _("Local models"), "models");
  adw_navigation_view_add (self->nav, root_nav);
  adw_navigation_view_add (self->nav, providers_nav);
  adw_navigation_view_add (self->nav, models_nav);

  /* Show the safe default until the broker proxy resolves, then read
   * real state (or keep the broker-absent fallback). */
  rebuild_providers (self, NULL, TRUE);
  rebuild_consent (self, NULL, TRUE);
  g_dbus_proxy_new_for_bus (G_BUS_TYPE_SESSION,
                            G_DBUS_PROXY_FLAGS_DO_NOT_AUTO_START, NULL,
                            REMOTE_BUS_NAME, REMOTE_OBJECT_PATH, REMOTE_IFACE,
                            self->cancellable, on_proxy_ready, self);

  adw_toast_overlay_set_child (self->toasts, GTK_WIDGET (self->nav));

  adw_navigation_page_set_title (ADW_NAVIGATION_PAGE (self), _("Intelligence"));
  adw_navigation_page_set_child (ADW_NAVIGATION_PAGE (self),
                                 GTK_WIDGET (self->toasts));
}
