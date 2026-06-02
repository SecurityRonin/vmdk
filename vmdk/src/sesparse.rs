//! seSparse (Space-Efficient Sparse) extent reader — vSphere 6.5+ VMFS6 snapshots.
//!
//! Detected by `SESPARSE` extent type in a text descriptor (not by file magic).
//! Two fixed 512-byte headers:
//!   - Constant header: magic `0x00000000CAFEBABE`
//!   - Volatile header: magic `0x00000000CAFECAFE`
//!
//! All fields are `u64` little-endian. GTEs are 8 bytes each.
//! Grain size MUST be 8 sectors (4 KiB). Grain table size MUST be 64 sectors
//! (= 4096 entries × 8 bytes ÷ 512 = 64 sectors per GT).
//!
//! Reference: QEMU `vmdk.c` `vmdk_open_se_sparse()`;
//! strict version check: `version == 0x0000_0002_0000_0001`.

use std::io::{self, Read, Seek, SeekFrom};

use crate::error::VmdkError;

/// Constant-header magic (`0x0000_0000_CAFE_BABE`, little-endian).
pub const SE_CONST_MAGIC: u64 = 0x0000_0000_CAFE_BABE;

/// Required version field in the constant header.
pub const SE_VERSION: u64 = 0x0000_0002_0000_0001;

/// Grain size in sectors — MUST be exactly 8 for seSparse.
pub const SE_GRAIN_SECTORS: u64 = 8;

/// Grain table size in sectors — MUST be exactly 64 (4096 entries × 8 B ÷ 512).
pub const SE_GT_SECTORS: u64 = 64;

/// Number of GTEs per grain table: 64 sectors × 512 bytes ÷ 8 bytes-per-GTE.
pub const SE_GTES_PER_GT: u64 = 4096;

const SECTOR_SIZE: u64 = 512;

/// Parsed seSparse constant header (first 512 bytes of the extent file).
pub(crate) struct SeConstHeader {
    pub capacity: u64,          // virtual disk size in sectors
    pub grain_size: u64,        // must be 8
    pub grain_table_size: u64,  // must be 64
    pub gd_offset: u64,         // grain directory sector offset
    pub gd_size: u64,           // grain directory size in sectors
    pub gt_offset: u64,         // start of grain tables (sectors)
    pub grains_offset: u64,     // start of grain data (sectors)
}

impl SeConstHeader {
    /// Parse the first 512 bytes of a seSparse extent file.
    pub fn parse(data: &[u8]) -> Result<Self, VmdkError> {
        if data.len() < 208 {
            return Err(VmdkError::FileTooSmall);
        }
        let magic = u64::from_le_bytes(data[0..8].try_into().expect("8 bytes"));
        if magic != SE_CONST_MAGIC {
            return Err(VmdkError::BadMagic);
        }
        let version = u64::from_le_bytes(data[8..16].try_into().expect("8 bytes"));
        if version != SE_VERSION {
            return Err(VmdkError::UnsupportedVersion(version as u32));
        }
        let capacity = u64::from_le_bytes(data[16..24].try_into().expect("8 bytes"));
        let grain_size = u64::from_le_bytes(data[24..32].try_into().expect("8 bytes"));
        if grain_size != SE_GRAIN_SECTORS {
            return Err(VmdkError::InvalidGeometry(
                format!("seSparse grain_size must be {SE_GRAIN_SECTORS}, got {grain_size}"),
            ));
        }
        let grain_table_size = u64::from_le_bytes(data[32..40].try_into().expect("8 bytes"));
        if grain_table_size != SE_GT_SECTORS {
            return Err(VmdkError::InvalidGeometry(
                format!("seSparse grain_table_size must be {SE_GT_SECTORS}, got {grain_table_size}"),
            ));
        }
        // Volatile header offset (sectors) at offset 80.
        // Grain directory offset at offset 128.
        let gd_offset = u64::from_le_bytes(data[128..136].try_into().expect("8 bytes"));
        let gd_size = u64::from_le_bytes(data[136..144].try_into().expect("8 bytes"));
        let gt_offset = u64::from_le_bytes(data[144..152].try_into().expect("8 bytes"));
        let grains_offset = u64::from_le_bytes(data[192..200].try_into().expect("8 bytes"));

        Ok(SeConstHeader { capacity, grain_size, grain_table_size, gd_offset, gd_size, gt_offset, grains_offset })
    }
}

/// Open a seSparse extent file, loading the grain directory into memory.
///
/// Returns `(grain_dir, grain_size_bytes, num_gtes_per_gt)`.
/// `grain_dir[i]` is the grain table index (not sector offset) for that GD slot,
/// or 0 if the slot is empty (all grains in that group are sparse).
pub(crate) fn open_sesparse<R: Read + Seek>(
    mut reader: R,
) -> Result<(Vec<u64>, u64, u64), VmdkError> {
    let mut hdr_bytes = [0u8; 512];
    reader.read_exact(&mut hdr_bytes)?;
    let hdr = SeConstHeader::parse(&hdr_bytes)?;

    let grain_size_bytes = hdr.grain_size * SECTOR_SIZE;

    // The number of GD entries = ceil(num_grains / GTES_PER_GT).
    let num_grains = (hdr.capacity + hdr.grain_size - 1) / hdr.grain_size;
    let num_gts = (num_grains + SE_GTES_PER_GT - 1) / SE_GTES_PER_GT;

    let gd_bytes = num_gts * 8; // 8 bytes per GD entry (u64)
    const MAX_SESP_GD: u64 = 16 * 1024 * 1024;
    if gd_bytes > MAX_SESP_GD {
        return Err(VmdkError::InvalidGeometry("seSparse grain directory too large".into()));
    }

    let gd_offset_bytes = hdr.gd_offset * SECTOR_SIZE;
    reader.seek(SeekFrom::Start(gd_offset_bytes))?;
    let mut buf = vec![0u8; gd_bytes as usize];
    reader.read_exact(&mut buf)?;

    let grain_dir = buf
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
        .collect();

    Ok((grain_dir, grain_size_bytes, SE_GTES_PER_GT))
}

/// Look up a GTE in a seSparse extent.
///
/// Returns the sector offset of the grain data, or 0 if the grain is unallocated.
/// seSparse GTEs are 8 bytes (u64) and store the grain sector offset directly.
pub(crate) fn sesparse_lookup_gte<R: Read + Seek>(
    reader: &mut R,
    grain_dir: &[u64],
    grain_size_bytes: u64,
    virtual_offset: u64,
    gt_offset_sectors: u64,
) -> io::Result<u64> {
    let grain_idx = virtual_offset / grain_size_bytes;
    let gd_idx = (grain_idx / SE_GTES_PER_GT) as usize;
    let gte_idx = grain_idx % SE_GTES_PER_GT;
    let gt_table_idx = grain_dir.get(gd_idx).copied().unwrap_or(0);
    if gt_table_idx == 0 {
        return Ok(0);
    }
    // GT table index is 0-based; actual sector = gt_offset_sectors + (gt_table_idx - 1) * SE_GT_SECTORS
    let gt_sector = gt_offset_sectors + (gt_table_idx - 1) * SE_GT_SECTORS;
    let gte_offset = gt_sector * SECTOR_SIZE + gte_idx * 8;
    reader.seek(SeekFrom::Start(gte_offset))?;
    let mut gte_bytes = [0u8; 8];
    reader.read_exact(&mut gte_bytes)?;
    Ok(u64::from_le_bytes(gte_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sesparse_header(capacity: u64) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[0..8].copy_from_slice(&SE_CONST_MAGIC.to_le_bytes());
        h[8..16].copy_from_slice(&SE_VERSION.to_le_bytes());
        h[16..24].copy_from_slice(&capacity.to_le_bytes());
        h[24..32].copy_from_slice(&SE_GRAIN_SECTORS.to_le_bytes()); // grain_size = 8
        h[32..40].copy_from_slice(&SE_GT_SECTORS.to_le_bytes());    // grain_table_size = 64
        // volatile header offset (80): just put 2
        h[80..88].copy_from_slice(&2u64.to_le_bytes());
        // gd_offset at 128
        h[128..136].copy_from_slice(&10u64.to_le_bytes()); // GD at sector 10
        // gd_size at 136
        h[136..144].copy_from_slice(&1u64.to_le_bytes());
        // gt_offset at 144
        h[144..152].copy_from_slice(&11u64.to_le_bytes());
        // grains_offset at 192
        h[192..200].copy_from_slice(&75u64.to_le_bytes());
        h
    }

    #[test]
    fn sesparse_header_parse_ok() {
        let h = make_sesparse_header(4096);
        let hdr = SeConstHeader::parse(&h).expect("parse");
        assert_eq!(hdr.capacity, 4096);
        assert_eq!(hdr.grain_size, 8);
        assert_eq!(hdr.grain_table_size, 64);
        assert_eq!(hdr.gd_offset, 10);
    }

    #[test]
    fn sesparse_wrong_magic_rejected() {
        let h = vec![0u8; 512];
        assert!(matches!(SeConstHeader::parse(&h), Err(VmdkError::BadMagic)));
    }

    #[test]
    fn sesparse_wrong_version_rejected() {
        let mut h = make_sesparse_header(8);
        h[8..16].copy_from_slice(&0u64.to_le_bytes()); // wrong version
        assert!(matches!(SeConstHeader::parse(&h), Err(VmdkError::UnsupportedVersion(_))));
    }

    #[test]
    fn sesparse_wrong_grain_size_rejected() {
        let mut h = make_sesparse_header(8);
        h[24..32].copy_from_slice(&16u64.to_le_bytes()); // grain_size=16, not 8
        assert!(matches!(SeConstHeader::parse(&h), Err(VmdkError::InvalidGeometry(_))));
    }
}
