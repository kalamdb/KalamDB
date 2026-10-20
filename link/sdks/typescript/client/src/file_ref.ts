/**
 * FileRef — Type-safe wrapper around the catalog `pg_catalog.file` composite type.
 */

import { KalamCellValue, wrapRowMap } from './cell_value.js';
import {
  fileRefDownloadUrl,
  fileRefRelativePath,
  fileRefRelativeUrl,
  fileRefStoredName,
} from '../wasm/kalam_client.js';
import type { RowData } from './cell_value.js';

function isWasmNotInitializedError(error: unknown): boolean {
  return error instanceof TypeError && error.message.includes('__wbindgen_add_to_stack_pointer');
}

function withFileRefFallback<T>(primary: () => T, fallback: () => T): T {
  try {
    return primary();
  } catch (error) {
    if (isWasmNotInitializedError(error)) {
      return fallback();
    }
    throw error;
  }
}

function sanitizeFilename(name: string): string {
  const nameWithoutExt = name.includes('.') ? name.slice(0, name.lastIndexOf('.')) : name;
  const sanitized = Array.from(nameWithoutExt)
    .flatMap((ch) => {
      if (/^[A-Za-z0-9]$/.test(ch)) return ch.toLowerCase();
      if (ch === ' ' || ch === '_' || ch === '-') return '-';
      return '';
    })
    .join('')
    .slice(0, 50);

  let result = '';
  let lastWasDash = true;
  for (const ch of sanitized) {
    if (ch === '-') {
      if (!lastWasDash) {
        result += ch;
      }
      lastWasDash = true;
    } else {
      result += ch;
      lastWasDash = false;
    }
  }

  return result.replace(/-+$/g, '');
}

function extractExtension(name: string): string {
  const dot = name.lastIndexOf('.');
  if (dot === -1) return 'bin';
  const ext = name.slice(dot + 1).replace(/[A-Z]/g, ch => ch.toLowerCase());
  return ext.length <= 10 && /^[a-z0-9]*$/.test(ext) ? ext : 'bin';
}

function storedNameFallback(data: PgCatalogFile): string {
  const sanitized = sanitizeFilename(data.name);
  const ext = extractExtension(data.name);
  return sanitized.length === 0 ? `${data.id}.${ext}` : `${data.id}-${sanitized}.${ext}`;
}

/**
 * Generated shape of the built-in `pg_catalog.file` composite type.
 */
export interface PgCatalogFile {
  id: string;
  sub: string;
  name: string;
  size: number | bigint;
  mime: string;
  sha256: string;
  shard?: number | null;
}

export type FileRefData = PgCatalogFile;

function isPgCatalogFile(value: unknown): value is PgCatalogFile {
  if (!value || typeof value !== 'object') {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  const validSize =
    (typeof candidate.size === 'number' && Number.isFinite(candidate.size))
    || typeof candidate.size === 'bigint';
  return typeof candidate.id === 'string'
    && typeof candidate.sub === 'string'
    && typeof candidate.name === 'string'
    && validSize
    && typeof candidate.mime === 'string'
    && typeof candidate.sha256 === 'string'
    && (candidate.shard === undefined || candidate.shard === null || typeof candidate.shard === 'number');
}

/**
 * Type-safe FileRef class wrapping the `pg_catalog.file` composite fields.
 */
export class FileRef implements PgCatalogFile {
  id: string;
  sub: string;
  name: string;
  size: number;
  mime: string;
  sha256: string;
  shard?: number;

  constructor(data: PgCatalogFile) {
    this.id = data.id;
    this.sub = data.sub;
    this.name = data.name;
    this.size = typeof data.size === 'bigint' ? Number(data.size) : data.size;
    this.mime = data.mime;
    this.sha256 = data.sha256;
    this.shard = data.shard ?? undefined;
  }

  /**
   * Parse FileRef from JSON string (as stored in FILE columns)
   */
  static fromJson(json: string | null | undefined): FileRef | null {
    if (!json) return null;

    try {
      const data = typeof json === 'string' ? JSON.parse(json) : json;
      return isPgCatalogFile(data) ? new FileRef(data) : null;
    } catch {
      return null;
    }
  }

  /**
   * Parse FileRef from unknown value (handles JSON string or object)
   */
  static from(value: unknown): FileRef | null {
    if (!value) return null;

    if (typeof value === 'string') {
      return FileRef.fromJson(value);
    }

    if (isPgCatalogFile(value)) {
      return new FileRef(value);
    }

    return null;
  }

  /**
   * Generate download URL for this file
   */
  getDownloadUrl(baseUrl: string, namespace: string, table: string): string {
    return withFileRefFallback(
      () => fileRefDownloadUrl(this.toJson(), baseUrl, namespace, table),
      () => `${baseUrl.replace(/\/+$/g, '')}/v1/files/${namespace}/${table}/${this.sub}/${storedNameFallback(this)}`,
    );
  }

  /**
   * Get the stored filename (with ID prefix)
   */
  storedName(): string {
    return withFileRefFallback(
      () => fileRefStoredName(this.toJson()),
      () => storedNameFallback(this),
    );
  }

  /**
   * Get the relative path within the table folder
   */
  relativePath(): string {
    return withFileRefFallback(
      () => fileRefRelativePath(this.toJson()),
      () => (this.shard === undefined
        ? `${this.sub}/${storedNameFallback(this)}`
        : `shard-${this.shard}/${this.sub}/${storedNameFallback(this)}`),
    );
  }

  /**
   * Format file size in human-readable format
   */
  formatSize(): string {
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let size = this.size;
    let unitIndex = 0;

    while (size >= 1024 && unitIndex < units.length - 1) {
      size /= 1024;
      unitIndex++;
    }

    return `${size.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`;
  }

  /**
   * Check if this is an image file (based on MIME type)
   */
  isImage(): boolean {
    return this.mime.startsWith('image/');
  }

  /**
   * Check if this is a video file (based on MIME type)
   */
  isVideo(): boolean {
    return this.mime.startsWith('video/');
  }

  /**
   * Check if this is an audio file (based on MIME type)
   */
  isAudio(): boolean {
    return this.mime.startsWith('audio/');
  }

  /**
   * Check if this is a PDF file
   */
  isPdf(): boolean {
    return this.mime === 'application/pdf';
  }

  /**
   * Get a user-friendly file type description
   */
  getTypeDescription(): string {
    if (this.isImage()) return 'Image';
    if (this.isVideo()) return 'Video';
    if (this.isAudio()) return 'Audio';
    if (this.isPdf()) return 'PDF Document';

    const parts = this.mime.split('/');
    if (parts.length === 2) {
      return parts[1].toUpperCase() + ' File';
    }

    return 'File';
  }

  /**
   * Serialize to JSON string
   */
  toJson(): string {
    return JSON.stringify({
      id: this.id,
      sub: this.sub,
      name: this.name,
      size: this.size,
      mime: this.mime,
      sha256: this.sha256,
      ...(this.shard !== undefined && { shard: this.shard }),
    });
  }

  /**
   * Convert to plain object
   */
  toObject(): PgCatalogFile {
    return {
      id: this.id,
      sub: this.sub,
      name: this.name,
      size: this.size,
      mime: this.mime,
      sha256: this.sha256,
      ...(this.shard !== undefined && { shard: this.shard }),
    };
  }
}

/**
 * Helper function to parse FileRef from query results
 *
 * @param value - Value from FILE column
 * @returns FileRef instance or null
 *
 * @example
 * ```typescript
 * const results = await client.query('SELECT * FROM users');
 * results.results[0].rows.forEach(row => {
 *   const avatar = parseFileRef(row.avatar);
 *   if (avatar) {
 *     console.log('Avatar URL:', avatar.getDownloadUrl(baseUrl, 'default', 'users'));
 *   }
 * });
 * ```
 */
export function parseFileRef(value: unknown): FileRef | null {
  return FileRef.from(value);
}

/**
 * Helper function to parse array of FileRefs from query results
 *
 * @param values - Array of values from FILE columns
 * @returns Array of FileRef instances (nulls filtered out)
 */
export function parseFileRefs(values: unknown[]): FileRef[] {
  return values
    .map((v) => FileRef.from(v))
    .filter((ref): ref is FileRef => ref !== null);
}

/* ================================================================== */
/*  Context-bound FileRef, Row, and Change wrappers                  */
/* ================================================================== */

/**
 * Context needed to generate file URLs without repeating server/table info.
 */
export interface FileRefContext {
  /** KalamDB server base URL (e.g., 'http://localhost:2900') */
  baseUrl: string;
  /** Namespace of the table (e.g., 'default') */
  namespace: string;
  /** Table name (e.g., 'users') */
  table: string;
}

/**
 * A `FileRef` with its server/table context already bound.
 *
 * Returned by `KalamRow.file()` — you never need to pass `baseUrl`,
 * `namespace`, or `table` again when generating URLs.
 *
 * @example
 * ```typescript
 * const rows = await client.queryRows<User>('SELECT * FROM default.users', 'default.users');
 * rows.forEach(row => {
 *   const avatar = row.file('avatar');
 *   if (avatar) {
 *     img.src = avatar.downloadUrl();        // full URL
 *     console.log(avatar.relativeUrl());     // /v1/files/default/users/f0001/12345-photo.jpg
 *     console.log(avatar.name, avatar.formatSize()); // all FileRef attrs still work
 *   }
 * });
 * ```
 */
export class BoundFileRef extends FileRef {
  private readonly _ctx: FileRefContext;

  constructor(data: PgCatalogFile, ctx: FileRefContext) {
    super(data);
    this._ctx = ctx;
  }

  /**
   * Full download URL — no args needed, context is already bound.
   */
  downloadUrl(): string {
    return withFileRefFallback(
      () => fileRefDownloadUrl(this.toJson(), this._ctx.baseUrl, this._ctx.namespace, this._ctx.table),
      () => `${this._ctx.baseUrl.replace(/\/+$/g, '')}/v1/files/${this._ctx.namespace}/${this._ctx.table}/${this.sub}/${storedNameFallback(this)}`,
    );
  }

  /**
   * Relative HTTP path — no args needed.
   */
  relativeUrl(): string {
    return withFileRefFallback(
      () => fileRefRelativeUrl(this.toJson(), this._ctx.namespace, this._ctx.table),
      () => `/v1/files/${this._ctx.namespace}/${this._ctx.table}/${this.sub}/${storedNameFallback(this)}`,
    );
  }
}

/**
 * A row with ergonomic FILE column access (SDK-level generic wrapper).
 *
 * This is inherently SDK-specific because `<T>` is a TypeScript generic that
 * cannot cross the Rust/WASM FFI boundary. Each SDK implements its own thin
 * wrapper; the heavy lifting (FileRef parsing, URL generation) delegates to
 * the shared `FileRef` class backed by the Rust implementation in `link-common`.
 *
 * - Access raw column values via `.data.columnName`
 * - Access any FILE column as a `BoundFileRef` via `.file('columnName')`
 *
 * @example
 * ```typescript
 * const rows = await client.queryRows<User>('SELECT * FROM default.users', 'default.users');
 * rows.forEach(row => {
 *   console.log(row.data.name);
 *   const avatar = row.file('avatar');
 *   if (avatar) img.src = avatar.downloadUrl();
 * });
 * ```
 */
export class KalamRow<T extends Record<string, unknown> = Record<string, unknown>> {
  /** Raw row data — all column values as returned from the server. */
  readonly data: T;
  private readonly _ctx: FileRefContext;
  private _typedData?: RowData;

  constructor(data: T, ctx: FileRefContext) {
    this.data = data;
    this._ctx = ctx;
  }

  /**
   * Access a column value as a `KalamCellValue` for type-safe extraction.
   *
   * Works identically whether the row came from a query or a subscription,
   * providing the unified `row.cell('colname').asString()` access pattern.
   *
   * @param column - Column name (type-checked against row type `T`)
   *
   * @example
   * ```typescript
   * // Query path:
   * const rows = await client.queryRows<User>('SELECT * FROM users', 'users');
   * rows.forEach(row => {
   *   console.log(row.cell('name').asString());
   *   console.log(row.cell('age').asInt());
   * });
   *
   * ```
   */
  cell(column: keyof T): KalamCellValue {
    const value = this.data[column];
    return value instanceof KalamCellValue ? value : KalamCellValue.from(value);
  }

  /**
   * All column values wrapped as `KalamCellValue` — cached on first access.
   *
   * Returns `RowData` (`Record<string, KalamCellValue>`) so you can use the
   * familiar `typedData['colname'].asString()` pattern.
   *
   * @example
   * ```typescript
   * const row = rows[0];
   * const td = row.typedData;
   * console.log(td['name'].asString(), td['score'].asFloat());
   * ```
   */
  get typedData(): RowData {
    if (!this._typedData) {
      const raw = this.data as Record<string, unknown>;
      const entries = Object.entries(raw);
      const alreadyWrapped = entries.every(([, value]) => value instanceof KalamCellValue);
      this._typedData = alreadyWrapped
        ? raw as unknown as RowData
        : wrapRowMap(raw);
    }
    return this._typedData!;
  }

  /**
   * Parse a FILE column and return a context-bound `BoundFileRef`.
   *
   * Returns `null` if the column is missing, null, or not a valid file reference.
   *
   * @param column - Column name (type-checked against row type `T`)
   *
   * @example
   * ```typescript
   * const avatar = row.file('avatar');
   * if (avatar) {
   *   img.src = avatar.downloadUrl();
   *   console.log(avatar.name, avatar.mime, avatar.formatSize());
   * }
   * ```
   */
  file(column: keyof T): BoundFileRef | null {
    const value = this.data[column];
    // Unwrap KalamCellValue instances to get the raw JSON for FileRef parsing
    const rawValue =
      value instanceof KalamCellValue ? value.toJson() : value;
    const ref = FileRef.from(rawValue);
    if (!ref) return null;
    return new BoundFileRef(ref.toObject(), this._ctx);
  }
}

/**
 * Wrap an array of raw row objects as `KalamRow<T>` with file access.
 *
 * @param rows - Raw row objects (from `queryAll()` or subscribe events)
 * @param ctx - Server/table context for file URL generation
 *
 * @example
 * ```typescript
 * const ctx = { baseUrl: 'http://localhost:2900', namespace: 'default', table: 'users' };
 * const rows = wrapRows<User>(queryResults, ctx);
 * const avatar = rows[0].file('avatar');
 * ```
 */
export function wrapRows<T extends Record<string, unknown> = Record<string, unknown>>(
  rows: T[],
  ctx: FileRefContext,
): KalamRow<T>[] {
  return rows.map(r => new KalamRow<T>(r, ctx));
}
