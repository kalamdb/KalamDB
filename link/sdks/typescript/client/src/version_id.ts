/**
 * Opaque row version (`VersionId`).
 *
 * The value is a signed 64-bit integer stored as `bigint`. It is not a
 * timestamp, worker id, or Snowflake. JSON uses the decimal string so values
 * above `2^53` do not round.
 *
 * @module
 */

/* ================================================================== */
/*  VersionId class                                                   */
/* ================================================================== */

/**
 * Opaque KalamDB row version.
 *
 * Wraps a signed 64-bit integer internally stored as `bigint`.
 * Instances are immutable and comparable.
 */
export class VersionId {
  /** @internal Raw value as bigint. */
  readonly #value: bigint;

  /* ------------------------------------------------------------------ */
  /*  Constructors                                                      */
  /* ------------------------------------------------------------------ */

  private constructor(value: bigint) {
    this.#value = value;
  }

  /**
   * Create a `VersionId` from a `bigint`, `number`, or decimal `string`.
   *
   * Throws if the value cannot be parsed or exceeds i64 range.
   */
  static from(value: bigint | number | string): VersionId {
    if (typeof value === 'bigint') return new VersionId(value);
    if (typeof value === 'number') {
      if (!Number.isFinite(value) || !Number.isInteger(value)) {
        throw new Error(`VersionId.from: expected integer, got ${value}`);
      }
      return new VersionId(BigInt(value));
    }
    if (typeof value === 'string') {
      const trimmed = value.trim();
      if (trimmed === '') throw new Error('VersionId.from: empty string');
      try {
        return new VersionId(BigInt(trimmed));
      } catch {
        throw new Error(`VersionId.from: failed to parse "${trimmed}"`);
      }
    }
    throw new Error(`VersionId.from: unsupported type ${typeof value}`);
  }

  /**
   * Create a `VersionId` from a raw WASM `number` value (the tsify-generated type).
   *
   * This handles the auto-generated `VersionId = number` from WASM bindings.
   * @internal
   */
  static fromWasm(value: number | undefined | null): VersionId | null {
    if (value === undefined || value === null) return null;
    return VersionId.from(value);
  }

  /** Construct with a zero value (useful for "start from beginning"). */
  static zero(): VersionId {
    return new VersionId(0n);
  }

  /* ------------------------------------------------------------------ */
  /*  Raw access                                                        */
  /* ------------------------------------------------------------------ */

  /** Return the raw value as `bigint`. */
  toBigInt(): bigint {
    return this.#value;
  }

  /** Return the raw value as `number`. May lose precision for large IDs. */
  toNumber(): number {
    return Number(this.#value);
  }

  /* ------------------------------------------------------------------ */
  /*  Comparison                                                        */
  /* ------------------------------------------------------------------ */

  /** Return `true` if this VersionId equals `other`. */
  equals(other: VersionId): boolean {
    return this.#value === other.#value;
  }

  /**
   * Compare to another VersionId.
   * Returns negative if `this < other`, zero if equal, positive if `this > other`.
   */
  compareTo(other: VersionId): number {
    if (this.#value < other.#value) return -1;
    if (this.#value > other.#value) return 1;
    return 0;
  }

  /* ------------------------------------------------------------------ */
  /*  Serialisation                                                     */
  /* ------------------------------------------------------------------ */

  /** Decimal string representation (for JSON / display). */
  toString(): string {
    return this.#value.toString();
  }

  /**
   * JSON form. A decimal string, matching the server `VersionId` encoding.
   */
  toJSON(): string {
    return this.toString();
  }
}
