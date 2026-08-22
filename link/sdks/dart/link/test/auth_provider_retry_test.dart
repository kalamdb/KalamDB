import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_link/kalam_link.dart';

void main() {
  group('AuthProvider retry', () {
    test('resolveAuthWithRetry retries transient errors then succeeds',
        () async {
      var attempts = 0;
      final delays = <Duration>[];

      final auth = await KalamClient.resolveAuthWithRetry(
        () async {
          attempts += 1;
          if (attempts < 3) {
            throw TimeoutException('network timeout');
          }
          return Auth.jwt('token-123');
        },
        maxAttempts: 4,
        initialBackoff: const Duration(milliseconds: 40),
        maxBackoff: const Duration(milliseconds: 200),
        sleep: (delay) async => delays.add(delay),
      );

      expect(attempts, 3);
      expect(auth, isA<JwtAuth>());
      expect((auth as JwtAuth).token, 'token-123');
      expect(delays, [
        const Duration(milliseconds: 40),
        const Duration(milliseconds: 80),
      ]);
    });

    test('resolveAuthWithRetry does not retry non-transient errors', () async {
      var attempts = 0;
      final delays = <Duration>[];

      await expectLater(
        KalamClient.resolveAuthWithRetry(
          () async {
            attempts += 1;
            throw StateError('invalid auth config');
          },
          maxAttempts: 5,
          initialBackoff: const Duration(milliseconds: 20),
          sleep: (delay) async => delays.add(delay),
        ),
        throwsA(isA<StateError>()),
      );

      expect(attempts, 1);
      expect(delays, isEmpty);
    });

    test('transient classifier recognizes common network failure text', () {
      expect(
        KalamClient.isLikelyTransientAuthProviderError(
          Exception('network-request-failed'),
        ),
        isTrue,
      );
      expect(
        KalamClient.isLikelyTransientAuthProviderError(
          Exception('503 Service Unavailable'),
        ),
        isTrue,
      );
      expect(
        KalamClient.isLikelyTransientAuthProviderError(
          Exception('bad credentials'),
        ),
        isFalse,
      );
    });
  });
}
