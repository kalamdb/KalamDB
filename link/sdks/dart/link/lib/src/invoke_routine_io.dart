import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'generated/api.dart';
import 'invoke_routine_error.dart';

export 'invoke_routine_error.dart';

final HttpClient _functionHttpClient = HttpClient();

/// Headers for a function invocation. A non-empty [descriptorRevision] is sent
/// as `x-kalam-descriptor-revision`.
Map<String, String> routineInvokeHeaders({
  required int epoch,
  String? descriptorRevision,
  required bool binary,
}) {
  final headers = <String, String>{
    'content-type': binary ? 'application/x-kalam-domain' : 'application/json',
    'accept': binary ? 'application/x-kalam-domain' : 'application/json',
    'x-kalam-epoch': '$epoch',
  };
  if (descriptorRevision != null && descriptorRevision.isNotEmpty) {
    headers['x-kalam-descriptor-revision'] = descriptorRevision;
  }
  return headers;
}

/// POST a function invocation. Binary is the default.
///
/// Frame bytes are produced by the shared Rust domain codec through
/// `dartEncodeRoutineRequest` / `dartDecodeRoutineResult`.
Future<Object?> postFunctionEnvelope({
  required String baseUrl,
  required String schema,
  required String name,
  required int epoch,
  required String? bearerToken,
  String? descriptorRevision,
  String format = 'binary',
  Map<String, Object?>? arguments,
}) async {
  final binary = format == 'binary';
  final uri = Uri.parse(
    '$baseUrl/v1/functions/${Uri.encodeComponent(schema)}/${Uri.encodeComponent(name)}',
  );
  final request = await _functionHttpClient.postUrl(uri);
  final headers = routineInvokeHeaders(
    epoch: epoch,
    descriptorRevision: descriptorRevision,
    binary: binary,
  );
  headers.forEach(request.headers.set);
  if (bearerToken != null) {
    request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $bearerToken');
  }
  request.add(binary
      ? dartEncodeRoutineRequest(
          argumentsJson: arguments == null ? 'null' : jsonEncode(_hostJson(arguments)),
          paramsJson: '[]',
        )
      : utf8.encode(jsonEncode({
          'epoch': epoch,
          'arguments': arguments,
        })));
  final response = await request.close();
  final bytes = await _readBytes(response);
  if (response.statusCode < 200 || response.statusCode >= 300) {
    throw routineExceptionFromPayload(response.statusCode, _tryJson(bytes));
  }
  final contentType = response.headers.contentType?.mimeType;
  if (contentType == 'application/x-kalam-domain' || contentType == 'application/octet-stream') {
    return jsonDecode(dartDecodeRoutineResult(frame: bytes));
  }
  return bytes.isEmpty ? null : jsonDecode(utf8.decode(bytes));
}

Future<Uint8List> _readBytes(HttpClientResponse response) async {
  final builder = BytesBuilder(copy: false);
  await for (final chunk in response) {
    builder.add(chunk);
  }
  return builder.toBytes();
}

Object? _hostJson(Object? value) {
  if (value is Uint8List) {
    return {'\$kbytes': base64Encode(value)};
  }
  if (value is Map) {
    return value.map((key, item) => MapEntry(key.toString(), _hostJson(item)));
  }
  if (value is List) {
    return value.map(_hostJson).toList();
  }
  return value;
}

Object? _tryJson(Uint8List bytes) {
  if (bytes.isEmpty) return null;
  try {
    return jsonDecode(utf8.decode(bytes));
  } on FormatException {
    return null;
  }
}
