import 'dart:io';

import 'package:kalam_sync_chat_backend/chat_backend.dart';

Future<void> main(List<String> args) async {
  final kalamUrl =
      Platform.environment['KALAMDB_URL'] ??
      Platform.environment['KALAM_URL'] ??
      'http://localhost:2900';
  final username =
      Platform.environment['KALAMDB_USER'] ??
      Platform.environment['KALAM_USER'] ??
      'root';
  final password =
      Platform.environment['KALAMDB_PASSWORD'] ??
      Platform.environment['KALAM_PASS'] ??
      'kalamdb123';
  final namespace = Platform.environment['KALAM_CHAT_NAMESPACE'] ?? 'chat';
  final port = int.parse(Platform.environment['KALAM_CHAT_PORT'] ?? '8787');
  final backend = ChatBackend(
    kalamUrl: kalamUrl,
    namespace: namespace,
    username: username,
    password: password,
    port: port,
  );
  final url = await backend.start();
  stdout.writeln('Chat backend listening on $url');
  stdout.writeln('KalamDB $kalamUrl namespace=$namespace user=$username');
  await ProcessSignal.sigint.watch().first;
  await backend.close();
}
