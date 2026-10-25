export 'invoke_routine_error.dart';

/// Web and other non-IO platforms have no native function bridge.
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
  throw UnsupportedError(
    'invokeRoutine requires a native Dart IO platform '
    '($baseUrl $schema.$name epoch $epoch, format $format, '
    'revision ${descriptorRevision ?? 'absent'}, '
    'auth ${bearerToken == null ? 'absent' : 'present'}, '
    'arguments ${arguments == null ? 'absent' : 'present'})',
  );
}
