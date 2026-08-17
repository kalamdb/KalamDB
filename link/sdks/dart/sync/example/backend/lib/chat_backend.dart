import 'dart:convert';
import 'dart:io';
import 'dart:math';

/// Demo HTTP engine that receives chat actions and writes authoritative rows
/// into KalamDB via `/v1/api/sql`.
final class ChatBackend {
  ChatBackend({
    required this.kalamUrl,
    required this.namespace,
    required this.username,
    required this.password,
    this.host = '127.0.0.1',
    this.port = 8787,
    HttpClient? httpClient,
    Random? random,
  }) : _httpClient = (httpClient ?? HttpClient())
         ..connectionTimeout = const Duration(seconds: 10)
         ..idleTimeout = const Duration(seconds: 15),
       _random = random ?? Random();

  final String kalamUrl;
  final String namespace;
  final String username;
  final String password;
  final String host;
  final int port;
  final HttpClient _httpClient;
  final Random _random;
  final Set<String> _idempotencyKeys = <String>{};
  HttpServer? _server;
  String? _accessToken;

  String get conversationsTable => '$namespace.conversations';
  String get messagesTable => '$namespace.messages';
  Uri get url => Uri.parse('http://$host:$port');

  static const cannedReplies = [
    'Got it. I will take a look.',
    'Nice — want me to expand on that?',
    'I saved that for later.',
    'Sounds good. Anything else?',
    'Here is a canned demo reply.',
  ];

  Future<Uri> start() async {
    await applySchema();
    _server = await HttpServer.bind(host, port);
    _server!.listen(_handle);
    return url;
  }

  Future<void> close() async {
    await _server?.close(force: true);
    _httpClient.close(force: true);
  }

  Future<void> applySchema() async {
    for (final statement in chatSchemaStatements(namespace)) {
      await executeSql(statement);
    }
  }

  Future<void> executeSql(String sql) async {
    final token = await _token();
    final request = await _httpClient.postUrl(
      Uri.parse('$kalamUrl/v1/api/sql'),
    );
    request.headers.contentType = ContentType.json;
    request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
    request.write(jsonEncode({'sql': sql}));
    final response = await request.close().timeout(
      const Duration(seconds: 15),
    );
    final body = await utf8.decodeStream(response);
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw HttpException(
        'KalamDB SQL HTTP ${response.statusCode}: $body\nSQL: $sql',
      );
    }
    if (body.isEmpty) return;
    final decoded = jsonDecode(body);
    if (decoded is Map && decoded['success'] == false) {
      final error = decoded['error']?.toString() ?? body;
      if (_isConflict(error)) return;
      throw HttpException('KalamDB SQL failed: $body\nSQL: $sql');
    }
  }

  bool _isConflict(String error) {
    final lower = error.toLowerCase();
    return lower.contains('duplicate') ||
        lower.contains('already exists') ||
        lower.contains('unique') ||
        lower.contains('primary key');
  }

  Future<bool> rowExists(String table, String id) async {
    final token = await _token();
    final sql =
        'SELECT id FROM $table WHERE id = ${sqlString(id)} LIMIT 1';
    final request = await _httpClient.postUrl(
      Uri.parse('$kalamUrl/v1/api/sql'),
    );
    request.headers.contentType = ContentType.json;
    request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
    request.write(jsonEncode({'sql': sql}));
    final response = await request.close().timeout(
      const Duration(seconds: 15),
    );
    final body = await utf8.decodeStream(response);
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw HttpException(
        'KalamDB lookup HTTP ${response.statusCode}: $body',
      );
    }
    final decoded = jsonDecode(body);
    if (decoded is! Map) return false;
    final results = decoded['results'];
    if (results is! List || results.isEmpty) return false;
    final rows = results.first is Map ? results.first['rows'] : null;
    if (rows is List) return rows.isNotEmpty;
    final named = results.first is Map
        ? results.first['named_rows'] ?? results.first['namedRows']
        : null;
    if (named is List) return named.isNotEmpty;
    final rowCount = results.first is Map ? results.first['row_count'] : null;
    if (rowCount is num) return rowCount > 0;
    return body.contains(id);
  }

  Future<void> _handle(HttpRequest request) async {
    request.response.headers.set('Access-Control-Allow-Origin', '*');
    request.response.headers.set(
      'Access-Control-Allow-Headers',
      'content-type, idempotency-key',
    );
    request.response.headers.set(
      'Access-Control-Allow-Methods',
      'GET, POST, OPTIONS',
    );
    try {
      if (request.method == 'OPTIONS') {
        request.response.statusCode = HttpStatus.noContent;
        await request.response.close();
        return;
      }
      final path = request.uri.path;
      if (request.method == 'GET' && path == '/health') {
        await _json(request, {'ok': true, 'namespace': namespace});
        return;
      }
      final body = await utf8.decodeStream(request);
      final json = body.isEmpty
          ? <String, Object?>{}
          : Map<String, Object?>.from(jsonDecode(body) as Map);
      if (request.method == 'POST' && path == '/v1/messages') {
        await persistMessage(json);
        await _json(request, {'ok': true});
        return;
      }
      final deliver = RegExp(r'^/v1/messages/([^/]+)/deliver$').firstMatch(path);
      if (request.method == 'POST' && deliver != null) {
        await markDelivered(
          Uri.decodeComponent(deliver.group(1)!),
          json,
        );
        await _json(request, {'ok': true});
        return;
      }
      final read = RegExp(r'^/v1/messages/([^/]+)/read$').firstMatch(path);
      if (request.method == 'POST' && read != null) {
        await markRead(Uri.decodeComponent(read.group(1)!), json);
        await _json(request, {'ok': true});
        return;
      }
      request.response.statusCode = HttpStatus.notFound;
      await _json(request, {'error': 'not found'});
    } catch (error) {
      request.response.statusCode = HttpStatus.internalServerError;
      await _json(request, {'error': error.toString()});
    }
  }

  Future<void> persistMessage(Map<String, Object?> json) async {
    final messageId = json['messageId']!.toString();
    final conversationId = json['conversationId']!.toString();
    final text = json['text']!.toString();
    final author = (json['author'] ?? 'user').toString();
    final createdAt = (json['createdAt'] ?? DateTime.now().toUtc().toIso8601String())
        .toString();
    final idempotencyKey = json['idempotencyKey']?.toString();
    if (idempotencyKey != null && !_idempotencyKeys.add(idempotencyKey)) {
      return;
    }
    if (await rowExists(messagesTable, messageId)) return;
    await _ensureConversation(conversationId);
    await executeSql(
      'INSERT INTO $messagesTable ('
      'id, conversation_id, role, author, text, status, created_at'
      ') VALUES ('
      '${sqlString(messageId)}, '
      '${sqlString(conversationId)}, '
      "'user', "
      '${sqlString(author)}, '
      '${sqlString(text)}, '
      "'sent', "
      '${sqlString(createdAt)}'
      ')',
    );
  }

  Future<void> markDelivered(
    String messageId,
    Map<String, Object?> json,
  ) async {
    final idempotencyKey = json['idempotencyKey']?.toString();
    if (idempotencyKey != null && !_idempotencyKeys.add(idempotencyKey)) {
      return;
    }
    final deliveredAt =
        (json['deliveredAt'] ?? DateTime.now().toUtc().toIso8601String())
            .toString();
    await executeSql(
      'UPDATE $messagesTable SET status = \'delivered\', '
      'delivered_at = ${sqlString(deliveredAt)} '
      'WHERE id = ${sqlString(messageId)}',
    );
    await _insertAssistantReply(messageId, json);
  }

  Future<void> markRead(String messageId, Map<String, Object?> json) async {
    final idempotencyKey = json['idempotencyKey']?.toString();
    if (idempotencyKey != null && !_idempotencyKeys.add(idempotencyKey)) {
      return;
    }
    final readAt =
        (json['readAt'] ?? DateTime.now().toUtc().toIso8601String()).toString();
    await executeSql(
      'UPDATE $messagesTable SET status = \'read\', '
      'read_at = ${sqlString(readAt)} '
      'WHERE id = ${sqlString(messageId)}',
    );
  }

  Future<void> _insertAssistantReply(
    String userMessageId,
    Map<String, Object?> json,
  ) async {
    final assistantId = 'ai-$userMessageId';
    if (await rowExists(messagesTable, assistantId)) return;
    final conversationId = json['conversationId']?.toString();
    if (conversationId == null || conversationId.isEmpty) {
      throw const FormatException('deliver requires conversationId');
    }
    final reply = cannedReplies[_random.nextInt(cannedReplies.length)];
    final createdAt = DateTime.now().toUtc().toIso8601String();
    await executeSql(
      'INSERT INTO $messagesTable ('
      'id, conversation_id, role, author, text, status, created_at, delivered_at'
      ') VALUES ('
      '${sqlString(assistantId)}, '
      '${sqlString(conversationId)}, '
      "'assistant', "
      "'demo-bot', "
      '${sqlString(reply)}, '
      "'delivered', "
      '${sqlString(createdAt)}, '
      '${sqlString(createdAt)}'
      ')',
    );
  }

  Future<void> _ensureConversation(String conversationId) async {
    if (await rowExists(conversationsTable, conversationId)) return;
    final now = DateTime.now().toUtc().toIso8601String();
    await executeSql(
      'INSERT INTO $conversationsTable (id, title, created_at, updated_at) '
      'VALUES ('
      '${sqlString(conversationId)}, '
      "'Chat', "
      '${sqlString(now)}, '
      '${sqlString(now)}'
      ')',
    );
  }

  Future<String> _token() async {
    final cached = _accessToken;
    if (cached != null) return cached;
    final request = await _httpClient.postUrl(
      Uri.parse('$kalamUrl/v1/api/auth/login'),
    );
    request.headers.contentType = ContentType.json;
    request.write(
      jsonEncode({'username': username, 'password': password}),
    );
    final response = await request.close().timeout(
      const Duration(seconds: 15),
    );
    final body = await utf8.decodeStream(response);
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw HttpException('KalamDB login HTTP ${response.statusCode}: $body');
    }
    final decoded = jsonDecode(body) as Map;
    final token =
        decoded['access_token'] ?? decoded['accessToken'] ?? decoded['token'];
    if (token is! String || token.isEmpty) {
      throw HttpException('KalamDB login missing access token: $body');
    }
    _accessToken = token;
    return token;
  }

  Future<void> _json(HttpRequest request, Map<String, Object?> body) async {
    request.response.headers.contentType = ContentType.json;
    request.response.write(jsonEncode(body));
    await request.response.close();
  }
}

List<String> chatSchemaStatements(String namespace) {
  return [
    'CREATE NAMESPACE IF NOT EXISTS $namespace',
    'CREATE TABLE IF NOT EXISTS $namespace.conversations ('
        'id TEXT PRIMARY KEY, '
        'title TEXT NOT NULL, '
        'created_at TIMESTAMP NOT NULL, '
        'updated_at TIMESTAMP NOT NULL'
        ") WITH (TYPE = 'USER')",
    'CREATE TABLE IF NOT EXISTS $namespace.messages ('
        'id TEXT PRIMARY KEY, '
        'conversation_id TEXT NOT NULL, '
        'role TEXT NOT NULL, '
        'author TEXT NOT NULL, '
        'text TEXT NOT NULL, '
        'status TEXT NOT NULL, '
        'created_at TIMESTAMP NOT NULL, '
        'delivered_at TIMESTAMP, '
        'read_at TIMESTAMP'
        ") WITH (TYPE = 'USER')",
  ];
}

String sqlString(String value) => "'${value.replaceAll("'", "''")}'";
