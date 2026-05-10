import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_link/kalam_link.dart';
import 'package:kalam_link/src/generated/frb_generated.dart';

const _fileRefDownloadUrl =
    'http://localhost:8080/v1/files/app/users/f0001/12345-avatar.png';
const _fileRefRelativeUrl = '/v1/files/app/users/f0001/12345-avatar.png';
const _fileRefStoredName = '12345-avatar.png';
const _fileRefRelativePath = 'f0001/12345-avatar.png';

void main() {
  setUpAll(() {
    RustLib.initMock(api: _FileRefRustApi());
  });

  tearDownAll(() {
    RustLib.dispose();
  });

  group('KalamFileRef', () {
    final sampleJson = jsonEncode({
      'id': '12345',
      'sub': 'f0001',
      'name': 'avatar.png',
      'size': 4096,
      'mime': 'image/png',
      'sha256': 'abc123',
    });

    test('parses JSON and delegates shared URL helpers', () {
      final ref = KalamFileRef.fromJson(sampleJson);
      expect(ref, isNotNull);
      expect(ref!.name, 'avatar.png');
      expect(ref.getDownloadUrl('http://localhost:8080', 'app', 'users'),
          _fileRefDownloadUrl);
      expect(ref.relativeUrl('app', 'users'), _fileRefRelativeUrl);
      expect(ref.storedName(), _fileRefStoredName);
      expect(ref.relativePath(), _fileRefRelativePath);
    });

    test('cell value asFileUrl uses the shared helper path', () {
      final cell = KalamCellValue(jsonDecode(sampleJson));
      expect(cell.asFileUrl('http://localhost:8080', 'app', 'users'),
          _fileRefDownloadUrl);
    });
  });
}

class _FileRefRustApi implements RustLibApi {
  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError(
        '_FileRefRustApi: ${invocation.memberName} not implemented',
      );

  @override
  String crateApiDartFileRefDownloadUrl({
    required String fileRefJson,
    required String baseUrl,
    required String namespace,
    required String table,
  }) {
    expect(fileRefJson, contains('"id":"12345"'));
    expect(baseUrl, 'http://localhost:8080');
    expect(namespace, 'app');
    expect(table, 'users');
    return _fileRefDownloadUrl;
  }

  @override
  String crateApiDartFileRefRelativeUrl({
    required String fileRefJson,
    required String namespace,
    required String table,
  }) {
    expect(fileRefJson, contains('"id":"12345"'));
    expect(namespace, 'app');
    expect(table, 'users');
    return _fileRefRelativeUrl;
  }

  @override
  String crateApiDartFileRefStoredName({required String fileRefJson}) {
    expect(fileRefJson, contains('"id":"12345"'));
    return _fileRefStoredName;
  }

  @override
  String crateApiDartFileRefRelativePath({required String fileRefJson}) {
    expect(fileRefJson, contains('"id":"12345"'));
    return _fileRefRelativePath;
  }
}
