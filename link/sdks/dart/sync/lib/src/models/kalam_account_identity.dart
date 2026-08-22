/// Cache and outbox ownership. A database must never cross this boundary.
final class KalamAccountIdentity {
  const KalamAccountIdentity({
    required this.serverUrl,
    required this.subject,
    this.namespace = 'default',
  });

  final String serverUrl;
  final String subject;
  final String namespace;

  String get accountKey => '$serverUrl|$namespace|$subject';

  String get databaseName => 'kalam_sync_${_fnv1a64(accountKey)}';
}

String _fnv1a64(String value) {
  var hash = 0xcbf29ce484222325;
  for (final byte in value.codeUnits) {
    hash ^= byte;
    hash = (hash * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF;
  }
  return hash.toRadixString(16).padLeft(16, '0');
}
