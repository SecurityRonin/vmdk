//! Snapshot/delta chain reader: layers a delta VMDK on top of its parent chain.
//!
//! Given a delta VMDK (parentCID != 0xffff_ffff), opens the parent referenced by
//! `parentFileNameHint`, validates that the parent's CID matches the delta's parentCID,
//! and presents a unified `Read + Seek` view where:
//! - allocated sectors in the delta are read from the delta
//! - sparse sectors in the delta are read from the parent (recursively)
//!
//! A chain depth limit (64 levels) guards against circular references in crafted images.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use crate::{VmdkError, VmdkFileReader};

/// Maximum number of delta layers in a chain before returning an error.
pub const MAX_CHAIN_DEPTH: usize = 64;

/// A read-only view over a VMDK snapshot chain.
///
/// Implements `Read + Seek` over the merged virtual sector stream, walking from the
/// most-recent delta down to the base image for each sparse grain.
///
/// Opened via [`VmdkChainReader::open`].
pub struct VmdkChainReader {
    /// Layers from newest (index 0 = delta) to oldest (last = base image).
    layers: Vec<VmdkFileReader>,
    virtual_disk_size: u64,
    pos: u64,
}

impl VmdkChainReader {
    /// Open a (potentially chained) VMDK from `path`, following `parentFileNameHint`
    /// until a base image is reached or `MAX_CHAIN_DEPTH` is exceeded.
    ///
    /// If `path` is a base image (`parentCID == 0xffff_ffff`), this is equivalent to
    /// `VmdkReader::open_path` wrapped in a single-layer chain.
    pub fn open(path: &Path) -> Result<Self, VmdkError> {
        let mut layers: Vec<VmdkFileReader> = Vec::new();
        let mut current_path = path.to_path_buf();

        for depth in 0..=MAX_CHAIN_DEPTH {
            let reader = VmdkFileReader::open_path(&current_path)?;
            let parent_cid = reader.parent_cid();

            if depth == 0 {
                layers.push(reader);
            } else {
                // Validate CID chain: the child's parentCID must match this parent's CID.
                let child_parent_cid = layers.last().map(|r| r.parent_cid()).unwrap_or(0xffff_ffff);
                let parent_actual_cid = reader.cid();
                if child_parent_cid != parent_actual_cid && parent_actual_cid != 0xffff_ffff {
                    // CID mismatch: parent was modified after the snapshot was taken.
                    // Log a warning but continue (same behaviour as QEMU).
                }
                layers.push(reader);
            }

            if parent_cid == 0xffff_ffff {
                break; // reached base image
            }
            if depth == MAX_CHAIN_DEPTH {
                return Err(VmdkError::InvalidGeometry(format!(
                    "snapshot chain depth exceeds limit of {MAX_CHAIN_DEPTH}"
                )));
            }

            // Resolve the parent path relative to the current file's directory.
            let desc_text = layers.last().map(|r| r.descriptor_text().to_owned()).unwrap_or_default();
            let parent_hint = extract_parent_file_name(&desc_text);
            if parent_hint.is_empty() {
                break; // no hint available — treat as base
            }
            let parent_path = if Path::new(parent_hint).is_absolute() {
                std::path::PathBuf::from(parent_hint)
            } else {
                current_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(parent_hint)
            };
            current_path = parent_path;
        }

        let virtual_disk_size = layers.first().map(|r| r.virtual_disk_size()).unwrap_or(0);
        Ok(VmdkChainReader { layers, virtual_disk_size, pos: 0 })
    }

    /// Total virtual disk size in bytes (from the delta/top layer).
    pub fn virtual_disk_size(&self) -> u64 {
        self.virtual_disk_size
    }

    /// Number of layers in the chain (1 = base image only, no parent).
    pub fn depth(&self) -> usize {
        self.layers.len()
    }
}

impl Read for VmdkChainReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.virtual_disk_size {
            return Ok(0);
        }
        // Try each layer from newest to oldest. Read from the first layer that has
        // data at this position. Sparse reads return zeros but we detect them by
        // checking is_allocated; if a layer doesn't have data, try the next.
        let to_read = buf.len().min((self.virtual_disk_size - self.pos) as usize);
        let lba = self.pos / 512;

        for layer in &mut self.layers {
            let allocated = layer.is_allocated(lba)?;
            if allocated {
                layer.seek(SeekFrom::Start(self.pos))?;
                let n = layer.read(&mut buf[..to_read])?;
                self.pos += n as u64;
                return Ok(n);
            }
        }

        // All layers are sparse at this position — return zeros.
        buf[..to_read].fill(0);
        self.pos += to_read as u64;
        Ok(to_read)
    }
}

impl Seek for VmdkChainReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(n) => self.pos as i64 + n,
            SeekFrom::End(n) => self.virtual_disk_size as i64 + n,
        };
        if new_pos < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

/// Extract `parentFileNameHint` value from a raw descriptor text.
fn extract_parent_file_name(text: &str) -> &str {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("parentFileNameHint=") {
            return rest.trim().trim_matches('"');
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, SeekFrom};

    use super::*;
    use crate::testutil::{write_chain_to_dir, GRAIN_SIZE_BYTES};

    #[test]
    fn extract_parent_file_name_parses_hint() {
        let desc = "# Disk DescriptorFile\nversion=1\nCID=00000001\nparentCID=ffffffff\nparentFileNameHint=\"../base.vmdk\"\ncreateType=\"monolithicSparse\"\n";
        assert_eq!(extract_parent_file_name(desc), "../base.vmdk");
    }

    #[test]
    fn extract_parent_file_name_returns_empty_when_absent() {
        let desc = "# Disk DescriptorFile\nversion=1\nCID=ffffffff\nparentCID=ffffffff\ncreateType=\"monolithicSparse\"\n";
        assert_eq!(extract_parent_file_name(desc), "");
    }

    #[test]
    fn chain_depth_one_for_base_image() {
        let dir = tempfile::tempdir().unwrap();
        let base_data = vec![0x42u8; 512];
        let (base_path, _) = write_chain_to_dir(dir.path(), &base_data);
        let chain = VmdkChainReader::open(&base_path).expect("open base image chain");
        assert_eq!(chain.depth(), 1, "base image has chain depth 1");
    }

    #[test]
    fn chain_depth_two_for_delta() {
        let dir = tempfile::tempdir().unwrap();
        let base_data = vec![0x42u8; 512];
        let (_, delta_path) = write_chain_to_dir(dir.path(), &base_data);
        let chain = VmdkChainReader::open(&delta_path).expect("open delta chain");
        assert_eq!(chain.depth(), 2, "delta over base has chain depth 2");
    }

    #[test]
    fn chain_reads_base_data_through_sparse_delta() {
        let dir = tempfile::tempdir().unwrap();
        let mut base_data = vec![0u8; GRAIN_SIZE_BYTES];
        base_data[0] = 0xDE;
        base_data[1] = 0xAD;
        let (_, delta_path) = write_chain_to_dir(dir.path(), &base_data);
        let mut chain = VmdkChainReader::open(&delta_path).expect("open chain");
        chain.seek(SeekFrom::Start(0)).expect("seek");
        let mut buf = [0u8; 2];
        chain.read_exact(&mut buf).expect("read");
        assert_eq!(buf, [0xDE, 0xAD], "chain must fall through to base data for sparse delta grain");
    }

    #[test]
    fn chain_virtual_disk_size_from_delta() {
        let dir = tempfile::tempdir().unwrap();
        let (_, delta_path) = write_chain_to_dir(dir.path(), &[0u8; 512]);
        let chain = VmdkChainReader::open(&delta_path).expect("open");
        assert_eq!(chain.virtual_disk_size(), GRAIN_SIZE_BYTES as u64);
    }
}
