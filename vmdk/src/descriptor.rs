//! Text VMDK descriptor parsing (Virtual Disk Format 1.1 §4.3).

use crate::error::{Result, VmdkError};

/// Parsed text VMDK descriptor.
pub(crate) struct TextDescriptor {
    pub create_type: Box<str>,
    pub extents: Vec<ExtentEntry>,
    /// Sum of all FLAT extent sector counts.
    pub capacity_sectors: u64,
}

/// A single flat extent entry from the descriptor.
pub(crate) struct ExtentEntry {
    pub size_sectors: u64,
    pub filename: Box<str>,
    /// Byte offset into the extent file where this extent's data begins.
    pub file_byte_offset: u64,
}

/// Parse a VMDK text descriptor, collecting `createType` and FLAT extents.
///
/// Non-FLAT extent types (SPARSE, ZERO, VMFS) are ignored; they require
/// separate per-extent binary VMDK parsing which is not yet implemented.
pub(crate) fn parse_text_descriptor(text: &str) -> Result<TextDescriptor> {
    let mut create_type = Box::from("");
    let mut extents = Vec::new();
    let mut capacity_sectors = 0u64;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("createType=") {
            create_type = Box::from(rest.trim_matches('"'));
            continue;
        }
        if let Some(ext) = try_parse_flat_extent(line) {
            capacity_sectors = capacity_sectors
                .checked_add(ext.size_sectors)
                .ok_or_else(|| VmdkError::InvalidGeometry("extent capacity overflow".into()))?;
            extents.push(ext);
        }
    }

    Ok(TextDescriptor {
        create_type,
        extents,
        capacity_sectors,
    })
}

/// Parse `<access> <n> FLAT "<file>" <sector_offset>` lines.
///
/// Returns `None` for non-FLAT types, blank lines, comment lines, or malformed input.
fn try_parse_flat_extent(line: &str) -> Option<ExtentEntry> {
    let mut rest = line;

    // Access token: RW | RDONLY (NOACCESS extents have no associated data)
    let (access, tail) = split_token(rest)?;
    if !matches!(access, "RW" | "RDONLY") {
        return None;
    }
    rest = tail;

    // Sector count
    let (sectors_str, tail) = split_token(rest)?;
    let size_sectors: u64 = sectors_str.parse().ok()?;
    rest = tail;

    // Extent type — only FLAT is supported here
    let (ext_type, tail) = split_token(rest)?;
    if ext_type != "FLAT" {
        return None;
    }
    rest = tail.trim_start();

    // Filename: bare word or double-quoted (may contain spaces)
    let (filename, remaining) = if let Some(inner) = rest.strip_prefix('"') {
        let close = inner.find('"')?;
        (&inner[..close], &inner[close + 1..])
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        (&rest[..end], &rest[end..])
    };

    let file_sector_offset: u64 = remaining.trim().parse().unwrap_or(0);

    Some(ExtentEntry {
        size_sectors,
        filename: Box::from(filename),
        file_byte_offset: file_sector_offset * 512,
    })
}

/// Split leading non-whitespace token from `s`; return `(token, rest_trimmed)`.
fn split_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    Some((&s[..end], &s[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_two_gb_max_extent_flat_descriptor() {
        let text = r#"# Disk DescriptorFile
version=1
CID=49bcfa17
parentCID=ffffffff
createType="twoGbMaxExtentFlat"

# Extent description
RW 2048 FLAT "flat-f001.vmdk" 0
"#;
        let d = parse_text_descriptor(text).expect("parse");
        assert_eq!(d.create_type.as_ref(), "twoGbMaxExtentFlat");
        assert_eq!(d.capacity_sectors, 2048);
        assert_eq!(d.extents.len(), 1);
        assert_eq!(d.extents[0].filename.as_ref(), "flat-f001.vmdk");
        assert_eq!(d.extents[0].size_sectors, 2048);
        assert_eq!(d.extents[0].file_byte_offset, 0);
    }

    #[test]
    fn parse_ignores_sparse_extents() {
        let text = "createType=\"monolithicSparse\"\nRW 128 SPARSE \"disk.vmdk\"\n";
        let d = parse_text_descriptor(text).expect("parse");
        assert_eq!(d.extents.len(), 0, "SPARSE extents must be ignored");
    }

    #[test]
    fn parse_quoted_filename_with_spaces() {
        let text = "createType=\"twoGbMaxExtentFlat\"\nRW 512 FLAT \"my disk.vmdk\" 0\n";
        let d = parse_text_descriptor(text).expect("parse");
        assert_eq!(d.extents[0].filename.as_ref(), "my disk.vmdk");
    }

    #[test]
    fn parse_nonzero_sector_offset() {
        let text = "createType=\"twoGbMaxExtentFlat\"\nRW 512 FLAT \"ext.vmdk\" 4096\n";
        let d = parse_text_descriptor(text).expect("parse");
        assert_eq!(d.extents[0].file_byte_offset, 4096 * 512);
    }
}
