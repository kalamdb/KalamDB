import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test('declared Flutter plugin platforms have matching directories', () {
    final root = Directory.current;
    final pubspecPath = File('${root.path}/pubspec.yaml');
    final pubspec = pubspecPath.readAsStringSync();

    final pluginDeclared = pubspec.contains('flutter:\n  plugin:');
    final declaredPlatforms = RegExp(r'^ {6}([a-z]+):\s*$', multiLine: true)
        .allMatches(pubspec)
        .map((match) => match.group(1)!)
        .toList(growable: false);

    if (!pluginDeclared) {
      expect(declaredPlatforms, isEmpty);
      return;
    }

    expect(declaredPlatforms, isNotEmpty);
    for (final dirName in declaredPlatforms) {
      final dir = Directory('${root.path}/$dirName');
      expect(
        dir.existsSync(),
        isTrue,
        reason: 'Missing plugin platform directory: $dirName',
      );
    }
  });

  test('android plugin bundles Rust bridge libraries', () {
    final root = Directory.current;
    final pubspecPath = File('${root.path}/pubspec.yaml');
    final pubspec = pubspecPath.readAsStringSync();

    final hasAndroidPlugin = RegExp(
      r'flutter:\s*\n\s*plugin:\s*\n\s*platforms:\s*\n\s*android:\s*\n\s*ffiPlugin:\s*true',
      multiLine: true,
    ).hasMatch(pubspec);

    if (!hasAndroidPlugin) {
      return;
    }

    final requiredAbiDirs = <String>['arm64-v8a', 'x86_64'];
    for (final abi in requiredAbiDirs) {
      final abiDir = Directory('${root.path}/android/src/main/jniLibs/$abi');
      final soFile = File('${abiDir.path}/libkalam_link_dart.so');
      expect(
        soFile.existsSync(),
        isTrue,
        reason: 'Missing Rust bridge library for ABI "$abi": ${soFile.path}',
      );
      final library = soFile.openSync();
      late Uint8List header;
      try {
        header = library.readSync(20);
      } finally {
        library.closeSync();
      }
      expect(header.length, 20, reason: 'Truncated native library for $abi');
      expect(header.sublist(0, 4), [0x7f, 0x45, 0x4c, 0x46],
          reason: 'Expected an ELF library for $abi');
      expect(header[4], 2, reason: 'Expected a 64-bit ELF library for $abi');
      expect(header[5], 1, reason: 'Expected little-endian ELF for $abi');
      final elf = ByteData.sublistView(header);
      expect(elf.getUint16(16, Endian.little), 3,
          reason: 'Expected a shared object for $abi');
      expect(elf.getUint16(18, Endian.little), abi == 'arm64-v8a' ? 183 : 62,
          reason: 'Native library architecture does not match $abi');
      final extras = abiDir
          .listSync()
          .whereType<File>()
          .where((file) =>
              file.path.endsWith('.so') &&
              !file.path.endsWith('libkalam_link_dart.so'))
          .map((file) => file.path)
          .toList(growable: false);
      expect(
        extras,
        isEmpty,
        reason:
            'jniLibs/$abi must only contain libkalam_link_dart.so; extras: $extras',
      );
    }
  });

  test('iOS static library stays under GitHub 50MB recommendation', () {
    final lib = File(
      '${Directory.current.path}/ios/Frameworks/libkalam_link_dart.a',
    );
    expect(lib.existsSync(), isTrue, reason: 'Missing ${lib.path}');
    const githubWarningBytes = 50 * 1024 * 1024;
    expect(
      lib.lengthSync(),
      lessThan(githubWarningBytes),
      reason:
          'libkalam_link_dart.a is ${lib.lengthSync()} bytes; GitHub warns above 50 MB. '
          'Rebuild with ./build.sh ios so bitcode/debug are stripped from the archive.',
    );
  });
}
