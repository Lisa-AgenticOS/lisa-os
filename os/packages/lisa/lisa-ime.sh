# Route text input through fcitx5 so the Lisa writing tools and the
# double-tap-Shift summon can see keys (PLAN §5.7.3 layer 2).
#
# WHY THIS FILE IS NECESSARY
# An input-method addon only receives keystrokes for surfaces whose
# toolkit is talking to that input method. GNOME defaults to IBus, so
# without these three variables fcitx5 runs, the addon loads, and not one
# key ever reaches it — the failure looks like broken code rather than
# unset configuration.
#
# GTK4 Wayland apps negotiate text-input-v3 with the compositor and
# ignore GTK_IM_MODULE; those work through mutter regardless. These
# variables are for everything else: GTK3, Qt, and XWayland clients,
# which between them are most of what people actually run.
export GTK_IM_MODULE=fcitx
export QT_IM_MODULE=fcitx
export XMODIFIERS=@im=fcitx
