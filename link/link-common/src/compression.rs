//! WebSocket message decompression utilities
//!
//! Provides gzip decompression for WebSocket messages received from the server.
//! The server compresses messages over 512 bytes automatically.

/// Maximum compressed input payload size (16 MiB).
pub const DEFAULT_MAX_COMPRESSED_BYTES: usize = 16 * 1024 * 1024;

/// Maximum decompressed output payload size (64 MiB).
pub const DEFAULT_MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

/// Check if data is gzip compressed (magic bytes check)
#[inline]
pub fn is_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b
}

/// Decompress gzip data using miniz_oxide (pure Rust, WASM-compatible)
/// with the bounded default output limit (`DEFAULT_MAX_DECOMPRESSED_BYTES`).
///
/// Returns the decompressed bytes, or an error if decompression fails.
pub fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, DecompressError> {
    decompress_gzip_with_limit(data, DEFAULT_MAX_DECOMPRESSED_BYTES)
}

/// Decompress gzip data with a maximum output size limit.
///
/// This prevents memory-exhaustion attacks from highly-compressed payloads.
pub fn decompress_gzip_with_limit(
    data: &[u8],
    max_output_bytes: usize,
) -> Result<Vec<u8>, DecompressError> {
    if data.len() < 18 {
        return Err(DecompressError::TooShort);
    }
    if data.len() > DEFAULT_MAX_COMPRESSED_BYTES {
        return Err(DecompressError::InputTooLarge {
            actual: data.len(),
            limit:  DEFAULT_MAX_COMPRESSED_BYTES,
        });
    }

    // Verify gzip magic bytes
    if !is_gzip(data) {
        return Err(DecompressError::NotGzip);
    }
    // Compression method 8 = DEFLATE; reject unknown methods or reserved flag bits (0xe0).
    let method = data[2];
    let flags = data[3];
    if method != 0x08 || (flags & 0xe0) != 0 {
        return Err(DecompressError::InvalidHeader);
    }

    let effective_limit = max_output_bytes.min(DEFAULT_MAX_DECOMPRESSED_BYTES);
    let mut pos = 10usize;

    // Skip extra field if present (FEXTRA flag = 0x04)
    if flags & 0x04 != 0 {
        if pos.checked_add(2).is_none_or(|end| end > data.len()) {
            return Err(DecompressError::InvalidHeader);
        }
        let xlen = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos = pos
            .checked_add(2)
            .and_then(|p| p.checked_add(xlen))
            .ok_or(DecompressError::InvalidHeader)?;
    }

    // Skip original filename if present (FNAME flag = 0x08)
    if flags & 0x08 != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        if pos >= data.len() {
            return Err(DecompressError::InvalidHeader);
        }
        pos += 1; // Skip null terminator
    }

    // Skip comment if present (FCOMMENT flag = 0x10)
    if flags & 0x10 != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        if pos >= data.len() {
            return Err(DecompressError::InvalidHeader);
        }
        pos += 1; // Skip null terminator
    }

    // Skip CRC16 if present (FHCRC flag = 0x02)
    if flags & 0x02 != 0 {
        pos = pos.checked_add(2).ok_or(DecompressError::InvalidHeader)?;
    }

    if pos > data.len() - 8 {
        return Err(DecompressError::InvalidHeader);
    }

    // The deflate data is between header and 8-byte trailer.
    let deflate_data = &data[pos..data.len() - 8];

    // ISIZE (last 4 bytes of the gzip trailer) is the uncompressed size modulo
    // 2^32. We use it as an early bound check.
    let advertised_size = u32::from_le_bytes([
        data[data.len() - 4],
        data[data.len() - 3],
        data[data.len() - 2],
        data[data.len() - 1],
    ]) as usize;
    if advertised_size > effective_limit {
        return Err(DecompressError::OutputTooLarge {
            advertised: advertised_size,
            limit:      effective_limit,
        });
    }

    // Gzip payloads carry raw RFC 1951 DEFLATE blocks; never fall back between formats.
    let decompressed =
        miniz_oxide::inflate::decompress_to_vec_with_limit(deflate_data, effective_limit)
            .map_err(|_| DecompressError::DecompressFailed)?;
    if (decompressed.len() as u32) != (advertised_size as u32) {
        return Err(DecompressError::DecompressFailed);
    }
    Ok(decompressed)
}

/// Decompress a frame if it carries gzip magic, or return the plain input borrowed.
///
/// Unlike permissive sniffers, if the payload starts with gzip magic (`0x1f 0x8b`)
/// and decompression fails or exceeds `max_output_bytes`, this returns `Err` rather
/// than falling back to the raw compressed bytes (`FR-057`, `T065`).
pub fn decompress_frame_checked(
    data: &[u8],
    max_output_bytes: usize,
) -> Result<std::borrow::Cow<'_, [u8]>, DecompressError> {
    if is_gzip(data) {
        decompress_gzip_with_limit(data, max_output_bytes).map(std::borrow::Cow::Owned)
    } else {
        if data.len() > max_output_bytes.min(DEFAULT_MAX_DECOMPRESSED_BYTES) {
            return Err(DecompressError::OutputTooLarge {
                advertised: data.len(),
                limit:      max_output_bytes.min(DEFAULT_MAX_DECOMPRESSED_BYTES),
            });
        }
        Ok(std::borrow::Cow::Borrowed(data))
    }
}

/// Decompress gzip data when valid, or return plain non-gzip input borrowed.
///
/// If `data` has gzip magic (`0x1f 0x8b`) and fails bounded decompression, returns an
/// empty slice rather than passing corrupted compressed bytes as plaintext (`T065`).
pub fn decompress_if_gzip(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    match decompress_frame_checked(data, DEFAULT_MAX_DECOMPRESSED_BYTES) {
        Ok(out) => out,
        Err(_) => std::borrow::Cow::Borrowed(&[]),
    }
}

/// Decompression error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecompressError {
    /// Data is too short to be valid gzip
    TooShort,
    /// Data doesn't have gzip magic bytes
    NotGzip,
    /// Invalid gzip header
    InvalidHeader,
    /// Decompression failed
    DecompressFailed,
    /// Compressed input exceeds configured limit
    InputTooLarge { actual: usize, limit: usize },
    /// Advertised decompressed payload exceeds configured limit
    OutputTooLarge {
        advertised: usize,
        limit:      usize,
    },
}

impl std::fmt::Display for DecompressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "Data too short for gzip"),
            Self::NotGzip => write!(f, "Not gzip compressed"),
            Self::InvalidHeader => write!(f, "Invalid gzip header"),
            Self::DecompressFailed => write!(f, "Decompression failed"),
            Self::InputTooLarge { actual, limit } => {
                write!(f, "Compressed payload too large ({} bytes > limit {} bytes)", actual, limit)
            },
            Self::OutputTooLarge { advertised, limit } => {
                write!(
                    f,
                    "Decompressed payload too large (advertised {} bytes > limit {} bytes)",
                    advertised, limit
                )
            },
        }
    }
}

impl std::error::Error for DecompressError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_gzip() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08]));
        assert!(!is_gzip(&[0x00, 0x00]));
        assert!(!is_gzip(&[0x1f]));
        assert!(!is_gzip(&[]));
    }

    #[test]
    fn test_decompress_if_gzip_plain() {
        let plain = b"Hello, World!";
        let result = decompress_if_gzip(plain);
        assert_eq!(&*result, plain);
    }

    #[test]
    fn test_decompress_if_gzip_plain_returns_borrowed() {
        // Non-gzip data should return a Cow::Borrowed (zero-copy).
        let plain = b"not compressed";
        let result = decompress_if_gzip(plain);
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn test_decompress_if_gzip_empty() {
        let result = decompress_if_gzip(&[]);
        assert!(result.is_empty());
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn test_decompress_gzip_with_limit_rejects_large_advertised_size() {
        // Minimal gzip payload with empty deflate body and a forged trailer
        // advertising a large uncompressed size.
        let mut payload = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0, 0x03];
        payload.extend_from_slice(&[0, 0, 0, 0]); // CRC32
        payload.extend_from_slice(&(1024u32).to_le_bytes()); // ISIZE

        let err =
            decompress_gzip_with_limit(&payload, 16).expect_err("must reject oversize payload");
        assert!(matches!(
            err,
            DecompressError::OutputTooLarge {
                advertised: 1024,
                limit:      16,
            }
        ));
    }

    #[test]
    fn t065_corrupt_gzip_never_falls_back_to_raw_bytes() {
        let corrupt_gzip = b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\x03deadbeef_not_deflate";
        assert!(decompress_frame_checked(corrupt_gzip, 1024).is_err());
        let fallback = decompress_if_gzip(corrupt_gzip);
        assert!(fallback.is_empty(), "corrupt gzip frame must never return raw compressed bytes");

        // Reserved flag bits (0xe0) must be rejected
        let bad_flags =
            b"\x1f\x8b\x08\x20\x00\x00\x00\x00\x00\x03\x03\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        assert_eq!(
            decompress_gzip_with_limit(bad_flags, 1024),
            Err(DecompressError::InvalidHeader)
        );
    }
}
