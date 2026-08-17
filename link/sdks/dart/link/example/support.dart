// ignore_for_file: avoid_print
library;

import 'dart:async';
import 'dart:io';

import 'package:kalam_link/kalam_link.dart';

String envOrDefault(List<String> keys, String fallback) {
  for (final key in keys) {
    final value = Platform.environment[key];
    if (value != null && value.trim().isNotEmpty) {
      return value;
    }
  }
  return fallback;
}

String get serverUrl =>
    envOrDefault(const ['KALAMDB_URL', 'KALAM_URL'], 'http://localhost:2900');

String get adminUser =>
    envOrDefault(const ['KALAMDB_USER', 'KALAM_USER'], 'admin');

String get adminPass => envOrDefault(
      const ['KALAMDB_PASSWORD', 'KALAM_PASS'],
      'kalamdb123',
    );

String sqlStringLiteral(String value) => "'${value.replaceAll("'", "''")}'";

class ExampleSession {
  ExampleSession({
    required this.login,
    required this.anonClient,
    required this.client,
  });

  final LoginResponse login;
  final KalamClient anonClient;
  final KalamClient client;

  Future<void> dispose() async {
    await client.dispose();
    await anonClient.dispose();
  }
}

Future<ExampleSession> connectAdminSession() async {
  await KalamClient.init();

  final anonClient = await KalamClient.connect(
    url: serverUrl,
    authProvider: () async => Auth.none(),
    timeout: const Duration(seconds: 10),
  );

  try {
    final login = await anonClient.login(adminUser, adminPass);
    final client = await KalamClient.connect(
      url: serverUrl,
      authProvider: () async => Auth.jwt(login.accessToken),
      timeout: const Duration(seconds: 10),
      maxRetries: 2,
    );
    return ExampleSession(
      login: login,
      anonClient: anonClient,
      client: client,
    );
  } catch (_) {
    await anonClient.dispose();
    rethrow;
  }
}

Future<QueryResponse> queryOrThrow(
  KalamClient client,
  String sql, {
  List<Object?>? params,
}) async {
  final response = await client.query(sql, params: params);
  if (!response.success) {
    throw StateError('Query failed for "$sql": ${response.error}');
  }
  return response;
}

Future<void> waitForCondition(
  FutureOr<bool> Function() predicate, {
  Duration timeout = const Duration(seconds: 15),
  Duration poll = const Duration(milliseconds: 150),
}) async {
  final deadline = DateTime.now().add(timeout);
  while (!await predicate()) {
    if (DateTime.now().isAfter(deadline)) {
      throw TimeoutException('Timed out waiting for example condition');
    }
    await Future<void>.delayed(poll);
  }
}

String describeRows(List<Map<String, KalamCellValue>> rows) {
  return rows.map(describeRow).join('\n');
}

String describeRow(Map<String, KalamCellValue> row) {
  final parts = row.entries.map((entry) {
    return '${entry.key}=${entry.value}';
  });
  return '{${parts.join(', ')}}';
}

String describeOptionalRow(Map<String, KalamCellValue>? row) {
  if (row == null) {
    return 'null';
  }
  return describeRow(row);
}