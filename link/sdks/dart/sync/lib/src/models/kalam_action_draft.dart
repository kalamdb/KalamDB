/// Immutable input used to create one durable action.
final class KalamActionDraft {
  const KalamActionDraft({
    required this.id,
    required this.accountKey,
    required this.actionKey,
    required this.payloadJson,
    this.version = 1,
    this.kind = 'custom',
    this.orderingKey,
  });

  final String id;
  final String accountKey;
  final String actionKey;
  final String payloadJson;
  final int version;
  final String kind;
  final String? orderingKey;
}
