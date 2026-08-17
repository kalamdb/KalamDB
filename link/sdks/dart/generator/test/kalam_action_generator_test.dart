import 'package:build/build.dart';
import 'package:build_test/build_test.dart';
import 'package:kalam_sync_generator/kalam_sync_generator.dart';
import 'package:test/test.dart';

void main() {
  test(
    'generates payload codec, stable definitions, and queue methods',
    () async {
      final reader = await _reader();
      await testBuilder(
        kalamActionBuilder(BuilderOptions.empty),
        const {
          'kalam_sync_generator|lib/messages.dart': '''
library messages;
import 'package:kalam_sync/kalam_sync.dart';
part 'messages.g.dart';

@KalamActionPayload()
class SendMessageArgs {
  const SendMessageArgs({
    required this.messageId,
    required this.text,
    required this.createdAt,
  });
  final String messageId;
  final String text;
  final DateTime createdAt;
}

@KalamActionModule(namespace: 'messages')
class MessageActions {
  @KalamAction(name: 'send', version: 2)
  Future<void> send(
    KalamActionContext context,
    SendMessageArgs payload,
  ) async {}
}
''',
        },
        outputs: {
          'kalam_sync_generator|lib/messages.kalam_actions.g.part':
              decodedMatches(
                allOf(
                  contains('SendMessageArgs _\$SendMessageArgsFromJson'),
                  contains("key: 'messages.send'"),
                  contains('version: 2'),
                  contains('final class MessageActionsQueue'),
                  contains('Future<KalamActionRecord> send('),
                  contains('actionId: actionId ?? Kalam.id()'),
                ),
              ),
        },
        readerWriter: reader,
        rootPackage: 'kalam_sync_generator',
        flattenOutput: true,
      );
    },
  );

  test('rejects payload fields that cannot be durably encoded', () async {
    final logs = <String>[];
    final reader = await _reader();
    await testBuilder(
      kalamActionBuilder(BuilderOptions.empty),
      const {
        'kalam_sync_generator|lib/invalid.dart': '''
library invalid;
import 'package:kalam_sync/src/annotations/kalam_action_payload.dart';
part 'invalid.g.dart';

@KalamActionPayload()
class InvalidPayload {
  const InvalidPayload({required this.callback});
  final void Function() callback;
}
''',
      },
      outputs: const {},
      onLog: (record) => logs.add(record.message),
      readerWriter: reader,
      rootPackage: 'kalam_sync_generator',
      flattenOutput: true,
    );
    expect(logs.join('\n'), contains('Unsupported durable payload field'));
  });

  test('rejects duplicate durable action keys', () async {
    final logs = <String>[];
    final reader = await _reader();
    await testBuilder(
      kalamActionBuilder(BuilderOptions.empty),
      const {
        'kalam_sync_generator|lib/duplicate.dart': '''
library duplicate;
import 'package:kalam_sync/src/actions/kalam_action_context.dart';
import 'package:kalam_sync/src/annotations/kalam_action.dart';
import 'package:kalam_sync/src/annotations/kalam_action_module.dart';
import 'package:kalam_sync/src/annotations/kalam_action_payload.dart';
part 'duplicate.g.dart';

@KalamActionPayload()
class Args {
  const Args({required this.id});
  final String id;
}

@KalamActionModule(namespace: 'messages')
class DuplicateActions {
  @KalamAction(name: 'send')
  Future<void> first(KalamActionContext context, Args payload) async {}

  @KalamAction(name: 'send')
  Future<void> second(KalamActionContext context, Args payload) async {}
}
''',
      },
      outputs: const {},
      onLog: (record) => logs.add(record.message),
      readerWriter: reader,
      rootPackage: 'kalam_sync_generator',
      flattenOutput: true,
    );
    expect(logs.join('\n'), contains('Duplicate action key'));
  });

  test(
    'generates independent queues for multiple table action modules',
    () async {
      final reader = await _reader();
      await testBuilder(
        kalamActionBuilder(BuilderOptions.empty),
        const {
          'kalam_sync_generator|lib/chat_actions.dart': '''
library chat_actions;
import 'package:kalam_sync/kalam_sync.dart';
part 'chat_actions.g.dart';

@KalamActionPayload()
class MessageArgs {
  const MessageArgs({required this.id});
  final String id;
}

@KalamActionPayload()
class ConversationArgs {
  const ConversationArgs({required this.id});
  final String id;
}

@KalamActionModule(namespace: 'messages')
class MessageActions {
  @KalamAction(name: 'create')
  Future<void> create(KalamActionContext context, MessageArgs payload) async {}

  @KalamAction(name: 'delete')
  Future<void> delete(KalamActionContext context, MessageArgs payload) async {}
}

@KalamActionModule(namespace: 'conversations')
class ConversationActions {
  @KalamAction(name: 'archive')
  Future<void> archive(
    KalamActionContext context,
    ConversationArgs payload,
  ) async {}
}
''',
        },
        outputs: {
          'kalam_sync_generator|lib/chat_actions.kalam_actions.g.part':
              decodedMatches(
                allOf(
                  contains("key: 'messages.create'"),
                  contains("key: 'messages.delete'"),
                  contains("key: 'conversations.archive'"),
                  contains('final class MessageActionsQueue'),
                  contains('final class ConversationActionsQueue'),
                ),
              ),
        },
        readerWriter: reader,
        rootPackage: 'kalam_sync_generator',
        flattenOutput: true,
      );
    },
  );
}

Future<TestReaderWriter> _reader() async {
  final reader = TestReaderWriter(
    rootPackage: 'kalam_sync_generator',
    flattenOutput: true,
  );
  await reader.testing.loadIsolateSources();
  return reader;
}
