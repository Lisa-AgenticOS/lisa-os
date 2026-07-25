/// lisa_ui — the widget kit Lisa apps import.
///
/// ADR-0014 (phase 1): lisa_ui is THE API surface for Lisa apps. It wraps
/// `package:flutter/material.dart` — a curated re-export of the Material
/// widget vocabulary plus Lisa-branded widgets and theming — so apps write
/// a single import:
///
/// ```dart
/// import 'package:lisa_ui/lisa_ui.dart';
/// ```
///
/// When Flutter finishes decoupling Material/Cupertino from the framework
/// core into packages, phase 2 swaps the backend to a vendored, re-themed
/// Material fork with no app-facing API change. ADR-0004 stays for the
/// history: it records the Flutter-lane decision and the original
/// core-widgets-only rule that this ADR supersedes for lisa_ui.
///
/// Theming: [lisaSeedColor] (violet) → `ColorScheme.fromSeed`, light +
/// dark, Material 3, Rubik as the default font family. Rubik is NOT
/// bundled and there is deliberately no google_fonts dependency — the
/// family name resolves against the OS-installed font and falls back to
/// the platform default sans when absent.
///
/// Tokens follow the elementary-inspired direction
/// (docs/notes/design-direction.md): restrained type, quiet color, humane
/// defaults. The theme file integration (Appendix E: shell + GTK + Qt +
/// Flutter all read one token source) replaces [LisaTokens.fallback] with
/// live values.
library;

import 'dart:async';

import 'package:flutter/material.dart';

/// The Material widget vocabulary, re-exported so apps need only this one
/// import. Curated: app structure, navigation, buttons, inputs, lists,
/// dialogs, feedback, and theming primitives. The Lisa-prefixed widgets
/// below are the pieces Material does not have.
export 'package:flutter/material.dart'
    show
        ActionChip,
        AlertDialog,
        Alignment,
        AlignmentGeometry,
        AppBar,
        AspectRatio,
        AutovalidateMode,
        Axis,
        BackButton,
        Badge,
        Border,
        BorderRadius,
        BorderSide,
        BoxConstraints,
        BoxDecoration,
        BoxFit,
        BoxShape,
        Brightness,
        BuildContext,
        Builder,
        Card,
        Center,
        Checkbox,
        CheckboxListTile,
        Chip,
        ChoiceChip,
        CircleAvatar,
        CircularProgressIndicator,
        Clip,
        ClipRRect,
        CloseButton,
        Color,
        ColorScheme,
        Colors,
        Column,
        ConstrainedBox,
        Container,
        CrossAxisAlignment,
        Curve,
        Curves,
        CustomScrollView,
        DecoratedBox,
        DefaultTabController,
        Dialog,
        Divider,
        Drawer,
        DropdownButton,
        DropdownMenu,
        DropdownMenuItem,
        EdgeInsets,
        EdgeInsetsGeometry,
        ElevatedButton,
        Expanded,
        FilledButton,
        FilterChip,
        FittedBox,
        Flex,
        Flexible,
        FloatingActionButton,
        FocusNode,
        FontStyle,
        FontWeight,
        Form,
        FormField,
        FutureBuilder,
        GestureDetector,
        GlobalKey,
        GridView,
        Hero,
        Icon,
        IconButton,
        IconData,
        Icons,
        Image,
        InkWell,
        InputBorder,
        InputChip,
        InputDecoration,
        Key,
        LayoutBuilder,
        LinearProgressIndicator,
        ListTile,
        ListView,
        MainAxisAlignment,
        MainAxisSize,
        Material,
        MaterialApp,
        MaterialPageRoute,
        MediaQuery,
        NavigationBar,
        NavigationDestination,
        NavigationRail,
        Navigator,
        Offset,
        Opacity,
        OutlinedButton,
        OutlineInputBorder,
        Padding,
        PageController,
        PageView,
        Placeholder,
        PopScope,
        PopupMenuButton,
        PopupMenuItem,
        Positioned,
        Radio,
        RadioListTile,
        Radius,
        RefreshIndicator,
        RichText,
        Row,
        SafeArea,
        // The entry point: without it no app can launch on the single
        // lisa_ui import (LisaCode writes `runApp(const LisaApp(...))`).
        runApp,
        Scaffold,
        ScaffoldMessenger,
        ScrollController,
        Scrollbar,
        SegmentedButton,
        SelectableText,
        showDatePicker,
        showDialog,
        showModalBottomSheet,
        showTimePicker,
        SimpleDialog,
        SimpleDialogOption,
        SingleChildScrollView,
        Size,
        SizedBox,
        Slider,
        SliverAppBar,
        SnackBar,
        SnackBarAction,
        Spacer,
        Stack,
        State,
        StatefulWidget,
        StatelessWidget,
        StreamBuilder,
        Switch,
        SwitchListTile,
        Tab,
        TabBar,
        TabBarView,
        Text,
        TextAlign,
        TextBaseline,
        TextButton,
        TextCapitalization,
        TextEditingController,
        TextField,
        TextFormField,
        TextInputAction,
        TextInputType,
        TextOverflow,
        TextScaler,
        TextStyle,
        TextTheme,
        Theme,
        ThemeData,
        ThemeMode,
        Tooltip,
        UnderlineInputBorder,
        ValueChanged,
        ValueListenableBuilder,
        VerticalDivider,
        VoidCallback,
        Widget,
        Wrap;

/// The violet seed every Lisa theme derives from (ADR-0014).
const Color lisaSeedColor = Color(0xFF6D45C9);

/// Builds the Lisa [ThemeData] for [brightness]: Material 3, the violet
/// seed color scheme, Rubik (OS-installed, platform sans fallback), and
/// [tokens] mapped into component shapes (card/dialog/input radius).
ThemeData lisaTheme(
  Brightness brightness, {
  LisaTokens tokens = LisaTokens.fallback,
}) {
  final scheme = ColorScheme.fromSeed(
    seedColor: lisaSeedColor,
    brightness: brightness,
  );
  final radius = BorderRadius.circular(tokens.radius);
  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    fontFamily: 'Rubik',
    cardTheme: CardThemeData(
      shape: RoundedRectangleBorder(borderRadius: radius),
    ),
    dialogTheme: DialogThemeData(
      shape: RoundedRectangleBorder(borderRadius: radius),
    ),
    inputDecorationTheme: InputDecorationTheme(
      border: OutlineInputBorder(borderRadius: radius),
    ),
  );
}

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

/// The root widget every Lisa app starts with: a [MaterialApp] pre-wired
/// to Lisa theming — [lisaTheme] light + dark, following the OS mode
/// unless [themeMode] says otherwise.
class LisaApp extends StatelessWidget {
  const LisaApp({
    super.key,
    required this.home,
    this.title = '',
    this.themeMode = ThemeMode.system,
    this.theme,
    this.darkTheme,
  });

  /// The app's home screen (usually a [LisaScaffold]).
  final Widget home;

  /// Passed through to [MaterialApp.title].
  final String title;

  /// Light/dark selection; defaults to following the OS.
  final ThemeMode themeMode;

  /// Overrides the [lisaTheme] light theme when set.
  final ThemeData? theme;

  /// Overrides the [lisaTheme] dark theme when set.
  final ThemeData? darkTheme;

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: title,
    theme: theme ?? lisaTheme(Brightness.light),
    darkTheme: darkTheme ?? lisaTheme(Brightness.dark),
    themeMode: themeMode,
    home: home,
  );
}

/// A [Scaffold] with Lisa defaults: an [AppBar] when [title] is set, and
/// the [body] inset by [SafeArea].
class LisaScaffold extends StatelessWidget {
  const LisaScaffold({
    super.key,
    required this.body,
    this.title,
    this.actions = const [],
    this.floatingActionButton,
  });

  final Widget body;

  /// When set, renders an [AppBar] with this title and [actions].
  final String? title;

  /// Trailing [AppBar] actions (ignored when [title] is null).
  final List<Widget> actions;

  final Widget? floatingActionButton;

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: title == null
        ? null
        : AppBar(title: Text(title!), actions: actions),
    body: SafeArea(child: body),
    floatingActionButton: floatingActionButton,
  );
}

/// A [Card] padded by the Lisa spacing token; the corner radius comes from
/// [lisaTheme]'s card shape (the radius token).
class LisaCard extends StatelessWidget {
  const LisaCard({super.key, required this.child, this.padding});

  final Widget child;

  /// Inner padding; defaults to [LisaTokens.spacing] on all sides.
  final EdgeInsetsGeometry? padding;

  @override
  Widget build(BuildContext context) {
    final t = LisaTheme.of(context);
    return Card(
      child: Padding(
        padding: padding ?? EdgeInsets.all(t.spacing),
        child: child,
      ),
    );
  }
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
