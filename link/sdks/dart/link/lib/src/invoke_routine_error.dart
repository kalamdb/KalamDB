/// Failure returned by `KalamClient.invokeRoutine`.
///
/// [code] is the server error code (`STALE_REVISION`, `EPOCH_MISMATCH`, and
/// the other function HTTP codes). [message] is the server message.
class KalamRoutineException implements Exception {
  const KalamRoutineException({
    required this.code,
    required this.message,
    required this.statusCode,
  });

  final String code;
  final String message;
  final int statusCode;

  @override
  String toString() => 'KalamRoutineException($code, $statusCode): $message';
}

KalamRoutineException routineExceptionFromPayload(int statusCode, Object? payload) {
  final record = payload is Map ? payload : const <String, Object?>{};
  final code = record['code']?.toString();
  final message = record['message']?.toString();
  return KalamRoutineException(
    code: code == null || code.isEmpty ? 'HTTP_$statusCode' : code,
    message: message == null || message.isEmpty
        ? 'function invoke failed: $statusCode'
        : message,
    statusCode: statusCode,
  );
}
