//! Little-endian decode helpers — one home for the byte-parsing patterns that
//! otherwise repeat across the format readers.

/// Little-endian `u32` from the first 4 bytes of `b` (panics if `b` is shorter).
#[inline]
pub(crate) fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b[..4].try_into().expect("4 bytes"))
}

/// Decode a packed table of little-endian `u32` entries (grain directory / table).
#[inline]
pub(crate) fn le_u32_table(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4).map(le_u32).collect()
}
