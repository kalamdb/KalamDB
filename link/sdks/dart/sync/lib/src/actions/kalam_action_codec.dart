/// Converts a typed action payload to and from durable JSON values.
final class KalamActionCodec<T> {
  const KalamActionCodec({required this.encode, required this.decode});

  final Map<String, Object?> Function(T payload) encode;
  final T Function(Map<String, Object?> json) decode;
}
