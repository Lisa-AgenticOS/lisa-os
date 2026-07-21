/// lisa_flutter — Dart bindings for the Lisa inference service.
///
/// PLAN §5.12: mirrors liblisa's surface. This first pass ships the
/// OpenAI-compat fallback transport (loopback HTTP against
/// lisa-inferenced) with zero package dependencies — `dart:io` only.
/// The primary D-Bus transport (sessions over org.lisa.Inference1 with
/// fd-passed streams, via package:dbus) lands with the Linux spike;
/// portal identity flows through it unchanged (ADR-0004).
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

/// Parse one OpenAI SSE line; returns the token delta, or null for
/// non-data lines, empty deltas, and `[DONE]`. Exposed for testing.
String? parseSseTokenLine(String line) {
  if (!line.startsWith('data: ')) return null;
  final data = line.substring(6).trim();
  if (data == '[DONE]' || data.isEmpty) return null;
  try {
    final json = jsonDecode(data) as Map<String, dynamic>;
    final choices = json['choices'] as List<dynamic>?;
    if (choices == null || choices.isEmpty) return null;
    final delta = (choices.first as Map<String, dynamic>)['delta']
        as Map<String, dynamic>?;
    final content = delta?['content'] as String?;
    return (content == null || content.isEmpty) ? null : content;
  } on FormatException {
    return null;
  }
}

/// Client for lisa-inferenced's OpenAI-compatible endpoint.
class LisaClient {
  LisaClient({Uri? base}) : base = base ?? Uri.parse('http://127.0.0.1:7777');

  final Uri base;
  final HttpClient _http = HttpClient();

  /// Stream token deltas for a prompt. Guided generation: pass a JSON
  /// Schema and the output is grammar-constrained server-side.
  Stream<String> ask(
    String prompt, {
    Map<String, dynamic>? jsonSchema,
    String? model,
    int? maxTokens,
  }) async* {
    final request =
        await _http.postUrl(base.replace(path: '/v1/chat/completions'));
    request.headers.contentType = ContentType.json;
    request.write(jsonEncode({
      'messages': [
        {'role': 'user', 'content': prompt},
      ],
      'stream': true,
      if (model != null) 'model': model,
      if (maxTokens != null) 'max_tokens': maxTokens,
      if (jsonSchema != null)
        'response_format': {
          'type': 'json_schema',
          'json_schema': {'name': 'schema', 'schema': jsonSchema},
        },
    }));
    final response = await request.close();
    if (response.statusCode != 200) {
      final body = await utf8.decodeStream(response);
      throw HttpException('lisa-inferenced returned ${response.statusCode}: '
          '${body.substring(0, body.length.clamp(0, 200))}');
    }
    await for (final line in response
        .transform(utf8.decoder)
        .transform(const LineSplitter())) {
      final token = parseSseTokenLine(line);
      if (token != null) yield token;
    }
  }

  /// Embed a text into a vector.
  Future<List<double>> embed(String text) async {
    final request = await _http.postUrl(base.replace(path: '/v1/embeddings'));
    request.headers.contentType = ContentType.json;
    request.write(jsonEncode({'input': text}));
    final response = await request.close();
    final body = await utf8.decodeStream(response);
    if (response.statusCode != 200) {
      throw HttpException(
          'lisa-inferenced returned ${response.statusCode}: $body');
    }
    final json = jsonDecode(body) as Map<String, dynamic>;
    final data = json['data'] as List<dynamic>;
    final embedding = (data.first as Map<String, dynamic>)['embedding']
        as List<dynamic>;
    return embedding.cast<num>().map((n) => n.toDouble()).toList();
  }

  void close() => _http.close(force: true);
}
