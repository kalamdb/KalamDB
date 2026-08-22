import 'kalam_action_codec.dart';
import 'kalam_action_context.dart';
import '../models/kalam_retry_policy.dart';

typedef KalamActionExecutor<T> =
    Future<void> Function(KalamActionContext context, T payload);

/// A typed, versioned durable action registered once by the application.
final class KalamActionDefinition<T> {
  const KalamActionDefinition({
    required this.key,
    required this.codec,
    required this.execute,
    this.version = 1,
    this.retryPolicy = const KalamRetryPolicy(),
  }) : assert(version > 0);

  final String key;
  final int version;
  final KalamActionCodec<T> codec;
  final KalamRetryPolicy retryPolicy;
  final KalamActionExecutor<T> execute;

  Map<String, Object?> encodePayload(Object? payload) {
    return codec.encode(payload as T);
  }

  Future<void> executePayload(
    KalamActionContext context,
    Map<String, Object?> json,
  ) {
    return execute(context, codec.decode(json));
  }
}
