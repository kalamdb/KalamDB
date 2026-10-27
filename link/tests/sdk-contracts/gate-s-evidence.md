# Gate S Evidence Record (T064–T068)

**Recorded:** 2026-10-23  
**Toolchain:** `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, `aarch64-apple-darwin`  
**Scope:** FR-032, FR-046, FR-049, FR-054, FR-057, FR-058.

## 1. Parsing-Time Preallocation, Depth, Work, and Arithmetic Bounds (`T064`)

- **Source:** `backend/crates/kalamdb-serialization/src/row/value.rs`, `row/batch.rs`, `row/decode.rs`
- **Hardened invariants:**
  1. `Reader` enforces parsing-time `MAX_NESTING = 32`, `MAX_LIST_LEN = 1_048_576`, `MAX_EMBEDDING_DIM = 65_536`, `MAX_DECODE_WORK = 2_097_152`, and `MAX_DECODED_BYTES = 64 MiB` before allocation or recursion.
  2. `decode_list` verifies `len <= MAX_LIST_LEN` and `len <= reader.remaining()` before charging work/byte budgets, and caps initial `Vec::with_capacity` to `MAX_PREALLOC_ELEMENTS = 1_024` so a 5-byte `u32::MAX` list frame cannot trigger multi-gigabyte preallocation.
  3. `decode_embedding` and `skip_value(TAG_EMBEDDING)` validate positive dimension bounds (`<= MAX_EMBEDDING_DIM`) and checked `dim * 4` byte lengths against `reader.remaining()` before allocation.
  4. `skip_value` charges work and nesting depth (`enter_nested` / `exit_nested`) on `TAG_STRUCT` and `TAG_LIST`, preventing stack overflow or CPU exhaustion in dropped columns.
  5. `row/batch.rs::assert_nesting` now recursively validates `depth + 1`, work budget, and child `ListArray::values().len()` rather than the outer 1-row `ListArray` length.
- **Executed test evidence:** `kalamdb-serialization` `row::batch::tests::t064_t067_rejects_huge_list_preallocation_deep_skip_and_bad_embedding_dims` — **PASS**.

## 2. Bounded Decompression Without Permissive Fallback (`T065`)

- **Source:** `link/link-common/src/compression.rs`, `link/kalam-link-wasm/src/helpers.rs`, `backend/crates/kalamdb-api/src/ws/compression.rs`
- **Hardened invariants:**
  1. `decompress_gzip` now enforces `DEFAULT_MAX_DECOMPRESSED_BYTES = 64 MiB` and `DEFAULT_MAX_COMPRESSED_BYTES = 16 MiB` (removing `usize::MAX`).
  2. Gzip headers require DEFLATE method `0x08`, reject reserved flag bits (`flags & 0xe0 != 0`), validate optional header fields with checked arithmetic, decompress raw RFC 1951 DEFLATE only (no zlib-to-raw-deflate fallback), and verify trailer `ISIZE`.
  3. `decompress_frame_checked` and `decompress_if_gzip` reject corrupt or oversized `0x1f 0x8b` gzip frames rather than returning raw compressed bytes as plaintext.
  4. Small payloads (`< 512` bytes) remain uncompressed (`COMPRESSION_THRESHOLD = 512`).
- **Executed test evidence:** `link-common` `compression::tests::t065_corrupt_gzip_never_falls_back_to_raw_bytes` and `test_decompress_gzip_with_limit_rejects_large_advertised_size` — **PASS**.

## 3. Native V8 Read-View Handle Ownership, Generation Fencing, and Buffer Allocator (`T066`)

- **Source:** `backend/crates/kalamdb-functions/src/abi/convert.rs`, `v8/adapter.rs`, `v8/async_ops.rs`, `v8/buffer_allocator.rs`
- **Hardened invariants:**
  1. Removed raw `Box::into_raw(Box::new(NativeReadViewHandle))` + `v8::External` (which leaked `NativeReadViewHandle` and its `Arc<[RecordBatch]>` on every view and `.at(i)` access).
  2. Replaced with isolate-scoped `ReadViewRegistry` (`MAX_READ_VIEW_HANDLES = 4_096`, `MAX_READ_VIEW_PINNED_BYTES = 16 MiB`) keyed by `[index, generation]` on private JS properties.
  3. `V8Session::invoke_with_host` and `V8Session::invoke_async` reset and remove `ReadViewRegistry` on every invocation, releasing all pinned `Arc<[RecordBatch]>` buffers deterministically and fencing stale generations across pooled isolate reuse.
  4. `BufferAllocator` tracks both active `used` and `peak` bytes against `limits.max_heap_bytes` with 16-byte aligned allocations.
- **Executed test evidence:** `kalamdb-functions` `abi::convert::tests::t066_read_view_registry_releases_pinned_batches_and_fences_generations` + all 851 `kalamdb-serialization` / `kalamdb-dialect` / `kalamdb-functions` tests — **PASS**.

## 4. Bounded Streaming Bodies Independent of `Content-Length` & Lockfile Review (`T068`)

- **HTTP streaming body bounds:** `backend/crates/kalamdb-api/src/http/sql/file_utils.rs` (`account_multipart_bytes`, `append_limited_field_chunk`) charges every received chunk against `MAX_MULTIPART_BUFFERED_BYTES` (`HTTP_PAYLOAD_BODY_LIMIT`), `MAX_MULTIPART_TEXT_FIELD_SIZE`, and `max_size_bytes` regardless of declared `Content-Length`. Tested by `test_append_limited_field_chunk_rejects_oversized_text_field` and `test_append_limited_field_chunk_rejects_total_buffer_growth`.
- **Workspace lockfile security floors (`Cargo.toml`):**
  - `aws-lc-rs = 1.18.2`, `rustls = 0.23.46`, `rustls-webpki = 0.103.15`, `quinn-proto = 0.11.19` pinned at workspace root.
  - `miniz_oxide = 0.9.1`, `flate2 = 1.1.10`, `v8 = 152.2.0`, `arrow >= 59.2.0`, `datafusion = 55.2.0`.
