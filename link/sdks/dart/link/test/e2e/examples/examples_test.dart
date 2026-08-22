library;

import 'package:flutter_test/flutter_test.dart';

import '../../../example/chat-app/main.dart' as chat_app;
import '../../../example/simple-events/main.dart' as simple_events;
import '../helpers.dart';

void main() {
  group('Examples', skip: skipIfNoIntegration, () {
    test('simple-events example runs end-to-end', () async {
      await simple_events.runSimpleEventsExample(
        namespace: uniqueName('sdk_example_simple'),
      );
    });

    test('chat-app example runs end-to-end', () async {
      final namespace = uniqueName('sdk_example_chat');
      final conversationId = uniqueName('conversation');
      await chat_app.runChatAppExample(
        namespace: namespace,
        conversationId: conversationId,
      );
    });
  });
}