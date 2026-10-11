/// Opaque row version. Not a timestamp or Snowflake.
///
/// Internally stores a 64-bit [int] (matching the Rust `VersionId` `i64`).
/// Use [toBigInt] when you need a [BigInt] representation.
///
/// ```dart
/// final version = VersionId.parse('123456789');
/// print(version.toString());
/// ```
class VersionId implements Comparable<VersionId> {

  /// The raw 64-bit integer value (matches Rust `i64` / FFI `PlatformInt64`).
  final int value;

  // --------------------------------------------------------------------
  //  Constructors
  // --------------------------------------------------------------------

  /// Create a [VersionId] from a raw [int] value.
  const VersionId(this.value);

  /// Create a [VersionId] with value zero (useful for "start from beginning").
  const VersionId.zero() : value = 0;

  /// Parse a [VersionId] from a decimal string, [int], [BigInt], or [double].
  ///
  /// Throws [FormatException] when the value cannot be converted.
  factory VersionId.parse(Object raw) {
    if (raw is int) return VersionId(raw);
    if (raw is BigInt) return VersionId(raw.toInt());
    if (raw is double) return VersionId(raw.truncate());
    if (raw is String) {
      final parsed = int.tryParse(raw);
      if (parsed != null) return VersionId(parsed);
      // Fallback: try BigInt for very large decimal strings then truncate.
      try {
        return VersionId(BigInt.parse(raw).toInt());
      } catch (_) {
        throw FormatException('VersionId.parse: failed to parse "$raw"');
      }
    }
    throw FormatException('VersionId.parse: unsupported type ${raw.runtimeType}');
  }

  /// Try to parse a [VersionId], returning `null` on failure.
  static VersionId? tryParse(Object? raw) {
    if (raw == null) return null;
    try {
      return VersionId.parse(raw);
    } catch (_) {
      return null;
    }
  }

  // --------------------------------------------------------------------
  //  Raw access
  // --------------------------------------------------------------------

  /// Return the raw value as a Dart [int].
  int toInt() => value;

  /// Return the raw value as a [BigInt].
  BigInt toBigInt() => BigInt.from(value);

  // --------------------------------------------------------------------
  //  Comparison
  // --------------------------------------------------------------------

  @override
  int compareTo(VersionId other) => value.compareTo(other.value);

  @override
  bool operator ==(Object other) => other is VersionId && value == other.value;

  @override
  int get hashCode => value.hashCode;

  /// Less than.
  bool operator <(VersionId other) => value < other.value;

  /// Less than or equal.
  bool operator <=(VersionId other) => value <= other.value;

  /// Greater than.
  bool operator >(VersionId other) => value > other.value;

  /// Greater than or equal.
  bool operator >=(VersionId other) => value >= other.value;

  // --------------------------------------------------------------------
  //  Serialisation / display
  // --------------------------------------------------------------------

  /// Decimal string representation (for JSON / display).
  @override
  String toString() => value.toString();
}

