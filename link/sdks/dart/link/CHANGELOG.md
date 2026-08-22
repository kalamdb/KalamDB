# Changelog

All notable changes to the `kalam_link` package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.3-alpha.4] - 2026-02-26

### Changed
- Bumped package version.
- Updated README (documented Alpha status, expanded docs/links).
- Removed broken Flutter plugin platform declaration from `pubspec.yaml`.
  The package did not include platform folders (`android/`, `ios/`, etc.),
  which caused Flutter plugin loader failures in consuming apps.

## [0.1.1] - 2026-02-26

### Fixed
- Removed dangling library doc comment warnings across e2e test files.
- Removed unused imports (`dart:io`, `dart:async`) and unused local variables in tests.

### Changed
- Updated README to be user-focused; moved developer/contributor notes to DEV.md.
- Added `publish.sh` for easy pub.dev publishing.

## [0.1.0] - 2026-02-26

### Added
- Initial release of `kalam_link` — KalamDB client SDK for Dart and Flutter.
- Query execution against KalamDB via HTTP/WebSocket.
- Live query subscriptions powered by `flutter_rust_bridge`.
- Authentication support (JWT bearer tokens).
- FFI plugin support for Android and iOS via `ffiPlugin`.
