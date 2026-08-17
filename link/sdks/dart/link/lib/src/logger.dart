// Logging utilities for the Kalam Link SDK.
//
// Provides structured logging with configurable levels and log listeners
// that can redirect SDK log output to Flutter's console, a file, or any
// other destination.
//
// ```dart
// // Enable debug logging to the Flutter console
// KalamLogger.level = Level.debug;
//
// // Redirect logs to your own handler
// KalamLogger.listener = (entry) {
//   myLogger.log(entry.level.name, entry.message);
// };
// ```

import 'package:logger/logger.dart' show Level;

export 'package:logger/logger.dart' show Level;

/// Backward-compatible alias for old SDK naming.
///
/// Prefer [Level] from `package:logger/logger.dart` in new code.
typedef LogLevel = Level;

/// A single log entry emitted by the SDK.
class LogEntry {
  /// Severity level.
  final Level level;

  /// Human-readable log message.
  final String message;

  /// The SDK component that produced this message (e.g. `'connection'`).
  final String tag;

  /// Timestamp when the log was created.
  final DateTime timestamp;

  const LogEntry({
    required this.level,
    required this.message,
    required this.tag,
    required this.timestamp,
  });

  @override
  String toString() =>
      '[${timestamp.toIso8601String()}] [kalam-link] [${level.name.toUpperCase()}] [$tag] $message';
}

/// Callback signature for receiving log entries.
///
/// Set via [KalamLogger.listener] to redirect SDK logs to your own handler.
typedef LogListener = void Function(LogEntry entry);

/// Global SDK logger.
///
/// Controls the minimum log level and an optional listener for capturing
/// log output. When no listener is set, messages are printed to `stdout`
/// via `print()`, which shows up in the Flutter/Android debug console
/// (`flutter run`, `adb logcat`, Android Studio Logcat, etc.).
///
/// ```dart
/// // Show everything in dev, silence in prod
/// KalamLogger.level = kDebugMode ? Level.debug : Level.warning;
///
/// // Forward to your logging framework
/// KalamLogger.listener = (entry) {
///   Fimber.log.d(entry.message, tag: entry.tag);
/// };
/// ```
class KalamLogger {
  KalamLogger._();

  /// Minimum severity level. Messages below this threshold are discarded.
  ///
  /// Defaults to [Level.warning] so that only warnings and errors appear
  /// unless the caller explicitly lowers the level.
  static Level level = Level.warning;

  /// Optional listener that receives every log entry at or above [level].
  ///
  /// When `null`, entries are printed via `print()` (visible in
  /// `flutter run`, `adb logcat`, Android Studio Logcat, VS Code debug
  /// console, etc.).
  static LogListener? listener;

  // ---------------------------------------------------------------------------
  // Internal API
  // ---------------------------------------------------------------------------

  static void _log(Level lvl, String tag, String message) {
    if (lvl < level) return;
    final entry = LogEntry(
      level: lvl,
      message: message,
      tag: tag,
      timestamp: DateTime.now(),
    );
    final cb = listener;
    if (cb != null) {
      cb(entry);
    } else {
      // ignore: avoid_print
      print(entry);
    }
  }

  /// Log at [Level.trace].
  static void verbose(String tag, String message) =>
      _log(Level.trace, tag, message);

  /// Log at [Level.debug].
  static void debug(String tag, String message) =>
      _log(Level.debug, tag, message);

  /// Log at [Level.info].
  static void info(String tag, String message) =>
      _log(Level.info, tag, message);

  /// Log at [Level.warning].
  static void warn(String tag, String message) =>
      _log(Level.warning, tag, message);

  /// Log at [Level.error].
  static void error(String tag, String message) =>
      _log(Level.error, tag, message);
}
