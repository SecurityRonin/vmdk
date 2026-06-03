//! Multi-extent flat VMDK reader: concatenates one or more raw extent files
//! into a single `Read + Seek` virtual sector stream.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::descriptor::ExtentEntry;

pub(crate) struct MultiExtentReader {
    extents: Vec<FlatExtent>,
    pos: u64,
    total_bytes: u64,
}

struct FlatExtent {
    /// First virtual byte this extent covers (inclusive).
    byte_start: u64,
    /// First virtual byte NOT covered by this extent.
    byte_end: u64,
    /// Byte offset in the extent file where this extent's data begins.
    file_offset: u64,
    /// `None` for a ZERO extent (no backing file — reads as zeros).
    file: Option<BufReader<File>>,
}

impl MultiExtentReader {
    pub(crate) fn open(base_dir: &Path, extents: &[ExtentEntry]) -> io::Result<Self> {
        let mut flat = Vec::with_capacity(extents.len());
        let mut virt = 0u64;
        for ext in extents {
            let size_bytes = ext.size_sectors * 512;
            // ZERO extents (and NOACCESS holes) have no backing file — they read as zeros.
            let file = if ext.is_zero {
                None
            } else {
                let path = base_dir.join(ext.filename.as_ref());
                Some(BufReader::new(File::open(&path).map_err(|e| {
                    io::Error::new(e.kind(), format!("{}: {e}", path.display()))
                })?))
            };
            flat.push(FlatExtent {
                byte_start: virt,
                byte_end: virt + size_bytes,
                file_offset: ext.file_byte_offset,
                file,
            });
            virt += size_bytes;
        }
        Ok(MultiExtentReader {
            extents: flat,
            pos: 0,
            total_bytes: virt,
        })
    }
}

impl Read for MultiExtentReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.total_bytes || buf.is_empty() {
            return Ok(0);
        }
        let ext = self
            .extents
            .iter_mut()
            .find(|e| e.byte_start <= self.pos && self.pos < e.byte_end)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "offset beyond extents"))?;
        let offset_in_extent = self.pos - ext.byte_start;
        let remaining_in_extent = (ext.byte_end - self.pos) as usize;
        let remaining_total = (self.total_bytes - self.pos) as usize;
        let to_read = buf.len().min(remaining_in_extent).min(remaining_total);
        let n = match &mut ext.file {
            Some(file) => {
                file.seek(SeekFrom::Start(ext.file_offset + offset_in_extent))?;
                file.read(&mut buf[..to_read])?
            }
            None => {
                // ZERO extent: emit zeros without touching disk.
                buf[..to_read].fill(0);
                to_read
            }
        };
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for MultiExtentReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(n) => self.pos as i64 + n,
            SeekFrom::End(n) => self.total_bytes as i64 + n,
        };
        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start",
            ));
        }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}
