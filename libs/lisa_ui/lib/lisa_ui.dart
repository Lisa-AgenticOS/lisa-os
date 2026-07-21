/// lisa_ui — the Lisa design system on Flutter *core* widgets.
///
/// PLAN §5.12 / ADR-0004: no `material_ui`/`cupertino_ui` dependency
/// anywhere in this lane — only `flutter/widgets.dart` primitives, so the
/// design system is ours from the first pixel. Tokens follow the
/// elementary-inspired direction (docs/notes/design-direction.md):
/// restrained type, quiet color, humane defaults. The theme file
/// integration (Appendix E: shell + GTK + Qt + Flutter all read one
/// token source) replaces [LisaTokens.fallback] with live values.
library;

import 'dart:async';

import 'package:flutter/widgets.dart';

/// Design tokens. `fallback` mirrors docs/notes/design-direction.md until
/// the system theme file lands; consumers must read tokens, never
/// hardcode.
class LisaTokens {
  const LisaTokens({
    required this.background,
    required this.surface,
    required this.textPrimary,
    required this.textSecondary,
    required this.accent,
    required this.danger,
    required this.radius,
    required this.spacing,
    required this.fontSize,
  });

  final Color background;
  final Color surface;
  final Color textPrimary;
  final Color textSecondary;
  final Color accent;
  final Color danger;
  final double radius;
  final double spacing;
  final double fontSize;

  static const fallback = LisaTokens(
    background: Color(0xFFFAFAF8),
    surface: Color(0xFFFFFFFF),
    textPrimary: Color(0xFF1A1A1E),
    textSecondary: Color(0xFF6A6A72),
    accent: Color(0xFF3A6EA5),
    danger: Color(0xFFB5443C),
    radius: 10,
    spacing: 12,
    fontSize: 15,
  );
}

/// Inherited access to the token set.
class LisaTheme extends InheritedWidget {
  const LisaTheme({super.key, required this.tokens, required super.child});

  final LisaTokens tokens;

  static LisaTokens of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<LisaTheme>()?.tokens ??
      LisaTokens.fallback;

  @override
  bool updateShouldNotify(LisaTheme oldWidget) => tokens != oldWidget.tokens;
}

/// Streaming model output: accumulates tokens as they arrive, shows a
/// stop affordance while streaming, and reserves the footnote row for
/// provenance chips (PLAN §5.12 `LisaStreamText`).
class LisaStreamText extends StatefulWidget {
  const LisaStreamText({
    super.key,
    required this.stream,
    this.onStop,
    this.provenance = const <String>[],
  });

  /// Token deltas (not full snapshots).
  final Stream<String> stream;

  /// Called when the user taps stop while streaming; null hides the
  /// affordance.
  final VoidCallback? onStop;

  /// Provenance labels rendered as footnotes (e.g. "file", "screen").
  final List<String> provenance;

  @override
  State<LisaStreamText> createState() => _LisaStreamTextState();
}

class _LisaStreamTextState extends State<LisaStreamText> {
  final StringBuffer _text = StringBuffer();
  StreamSubscription<String>? _sub;
  bool _done = false;

  @override
  void initState() {
    super.initState();
    _sub = widget.stream.listen(
      (token) => setState(() => _text.write(token)),
      onDone: () => setState(() => _done = true),
      onError: (_) => setState(() => _done = true),
    );
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = LisaTheme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          _text.toString(),
          style: TextStyle(
            color: t.textPrimary,
            fontSize: t.fontSize,
            height: 1.45,
          ),
        ),
        if (!_done && widget.onStop != null)
          Padding(
            padding: EdgeInsets.only(top: t.spacing / 2),
            child: GestureDetector(
              onTap: widget.onStop,
              child: Container(
                padding: EdgeInsets.symmetric(
                  horizontal: t.spacing,
                  vertical: t.spacing / 2,
                ),
                decoration: BoxDecoration(
                  border: Border.all(color: t.textSecondary),
                  borderRadius: BorderRadius.circular(t.radius),
                ),
                child: Text(
                  'Stop',
                  style: TextStyle(
                    color: t.textSecondary,
                    fontSize: t.fontSize - 2,
                  ),
                ),
              ),
            ),
          ),
        if (_done && widget.provenance.isNotEmpty)
          Padding(
            padding: EdgeInsets.only(top: t.spacing / 2),
            child: Text(
              widget.provenance.map((p) => '⌁ $p').join('   '),
              style: TextStyle(
                color: t.textSecondary,
                fontSize: t.fontSize - 3,
              ),
            ),
          ),
      ],
    );
  }
}

/// Consent affordance for a scope request (PLAN §5.12 `ConsentChip`):
/// states the scope plainly, offers allow / deny, never dark-patterns.
class ConsentChip extends StatelessWidget {
  const ConsentChip({
    super.key,
    required this.scope,
    required this.onAllow,
    required this.onDeny,
  });

  final String scope;
  final VoidCallback onAllow;
  final VoidCallback onDeny;

  @override
  Widget build(BuildContext context) {
    final t = LisaTheme.of(context);
    Widget action(String label, Color color, VoidCallback onTap) =>
        GestureDetector(
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.spacing,
              vertical: t.spacing / 2,
            ),
            child: Text(
              label,
              style: TextStyle(
                color: color,
                fontSize: t.fontSize - 1,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
        );

    return Container(
      padding: EdgeInsets.all(t.spacing / 2),
      decoration: BoxDecoration(
        color: t.surface,
        borderRadius: BorderRadius.circular(t.radius),
        border: Border.all(color: t.textSecondary.withValues(alpha: 0.4)),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Padding(
            padding: EdgeInsets.symmetric(horizontal: t.spacing / 2),
            child: Text(
              scope,
              style: TextStyle(color: t.textPrimary, fontSize: t.fontSize - 1),
            ),
          ),
          action('Allow', t.accent, onAllow),
          action('Deny', t.danger, onDeny),
        ],
      ),
    );
  }
}
