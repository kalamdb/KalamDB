import 'package:kalam_link/kalam_link.dart';

/// Last server sequence durably committed for one subscription.
final class KalamCheckpoint {
  const KalamCheckpoint({
    required this.accountKey,
    required this.subscriptionId,
    required this.seq,
    required this.updatedAt,
  });

  final String accountKey;
  final String subscriptionId;
  final SeqId seq;
  final DateTime updatedAt;
}
