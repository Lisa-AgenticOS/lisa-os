import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lisa_ui/lisa_ui.dart';

Widget harness(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: LisaTheme(tokens: LisaTokens.fallback, child: child),
    );

void main() {
  testWidgets('LisaStreamText accumulates streamed tokens', (tester) async {
    final controller = StreamController<String>();
    await tester.pumpWidget(harness(
      LisaStreamText(stream: controller.stream, onStop: () {}),
    ));

    controller.add('Hello ');
    await tester.pump();
    controller.add('world');
    await tester.pump();

    expect(find.textContaining('Hello world'), findsOneWidget);
    expect(find.text('Stop'), findsOneWidget, reason: 'streaming shows stop');

    await controller.close();
    await tester.pump();
    expect(find.text('Stop'), findsNothing, reason: 'done hides stop');
  });

  testWidgets('LisaStreamText renders provenance footnotes when done',
      (tester) async {
    final controller = StreamController<String>();
    await tester.pumpWidget(harness(
      LisaStreamText(
        stream: controller.stream,
        provenance: const ['file', 'screen'],
      ),
    ));
    controller.add('answer');
    await controller.close();
    await tester.pump();

    expect(find.textContaining('⌁ file'), findsOneWidget);
    expect(find.textContaining('⌁ screen'), findsOneWidget);
  });

  testWidgets('ConsentChip fires allow and deny callbacks', (tester) async {
    var allowed = 0;
    var denied = 0;
    await tester.pumpWidget(harness(ConsentChip(
      scope: 'documents.read',
      onAllow: () => allowed++,
      onDeny: () => denied++,
    )));

    expect(find.text('documents.read'), findsOneWidget);
    await tester.tap(find.text('Allow'));
    await tester.tap(find.text('Deny'));
    expect((allowed, denied), (1, 1));
  });
}
