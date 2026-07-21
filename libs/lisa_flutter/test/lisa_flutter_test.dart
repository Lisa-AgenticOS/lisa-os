import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lisa_flutter/lisa_flutter.dart';

void main() {
  group('SSE parsing', () {
    test('extracts token deltas from data lines', () {
      const line =
          'data: {"choices":[{"delta":{"content":"hello "},"index":0}]}';
      expect(parseSseTokenLine(line), 'hello ');
    });

    test('ignores role preambles, DONE, blanks, and junk', () {
      expect(
        parseSseTokenLine('data: {"choices":[{"delta":{"role":"assistant"}}]}'),
        isNull,
      );
      expect(parseSseTokenLine('data: [DONE]'), isNull);
      expect(parseSseTokenLine(''), isNull);
      expect(parseSseTokenLine(': keep-alive'), isNull);
      expect(parseSseTokenLine('data: not-json'), isNull);
    });
  });

  group('live round trip (skips when lisa-inferenced is down)', () {
    Future<bool> daemonUp() async {
      try {
        final http = HttpClient()
          ..connectionTimeout = const Duration(seconds: 1);
        final req = await http.getUrl(Uri.parse('http://127.0.0.1:7777/health'));
        final res = await req.close();
        await res.drain<void>();
        http.close(force: true);
        return res.statusCode == 200;
      } on SocketException {
        return false;
      } on HttpException {
        return false;
      }
    }

    test('ask streams and embed returns a vector', () async {
      if (!await daemonUp()) {
        markTestSkipped('lisa-inferenced not running on 127.0.0.1:7777');
        return;
      }
      final client = LisaClient();
      final text =
          (await client.ask('dart-transport-canary').toList()).join();
      expect(text, contains('dart-transport-canary'));

      final vector = await client.embed('embed me');
      expect(vector, isNotEmpty);
      client.close();
    });
  });
}
