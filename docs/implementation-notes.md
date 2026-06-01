# VMDK Implementation Notes

Developer notes capturing format quirks, spec contradictions, and empirically verified
behaviour. Intended for future contributors and as a basis for upstream spec clarifications.

Reference: *VMware Virtual Disk Format 1.1* (August 2011).

---

## 1. Sparse monolithic VMDK only

This implementation supports **monolithic sparse** VMDKs: a single file containing the
`SparseExtentHeader`, embedded text descriptor, grain directory, grain tables, and
grain data. Other VMDK variants are rejected:

| Variant | Status | Reason |
|---------|--------|--------|
| Monolithic sparse (hosted) | **Supported** | This implementation |
| Monolithic flat | Rejected (`CompressedNotSupported` ≠ correct error, but flat has no grain directory) | Out of scope |
| Two-GBmax sparse | Rejected | Multi-extent; extent descriptor parsing not implemented |
| Stream-optimized | Rejected | Different grain directory location and compression layout |
| VMFS sparse | Rejected | ESX-only; different header magic |

---

## 2. Two-level grain directory: GD → GT → GTE

VMDKs use a two-level indirection table:

```
Grain Directory (GD)         ← indexed by grain_dir_idx = grain_idx / num_gtes_per_gt
  └── Grain Table (GT)       ← read from file; indexed by gte_idx = grain_idx % num_gtes_per_gt
        └── Grain Table Entry (GTE) ← 32-bit value; 0 or 1 = sparse; >1 = sector offset
```

All offsets are in **sectors** (512 bytes). To resolve a byte offset:

```rust
let grain_idx        = virtual_offset / grain_size_bytes;
let offset_in_grain  = virtual_offset % grain_size_bytes;
let gd_idx           = grain_idx / num_gtes_per_gt;
let gte_idx          = grain_idx % num_gtes_per_gt;
let gt_sector        = grain_dir[gd_idx];          // loaded at open time
let gte_file_pos     = gt_sector * 512 + gte_idx * 4;
// read 4-byte GTE from file at gte_file_pos
let file_offset      = gte * 512 + offset_in_grain; // gte is the grain's sector
```

### Grain directory is eagerly loaded; grain tables are on-demand

The grain directory is small (one 32-bit entry per grain table, typically a few KB)
and is loaded entirely at open time. Grain tables are read on demand per-read to
avoid loading potentially megabytes of tables at open time.

---

## 3. GTE values 0 and 1: both mean "sparse, return zeros"

From the spec:

| GTE value | Meaning |
|-----------|---------|
| 0 | Not allocated / sparse — read as zeros |
| 1 | Zeroed grain — read as zeros |
| ≥ 2 | File sector offset of the grain data |

**Common pitfall:** treating only GTE = 0 as sparse. GTE = 1 is a valid "explicitly
zeroed" state used by some VMware tools; returning a file read at sector 1 (file
offset 512) yields wrong data.

```rust
if gte <= 1 {
    return Ok(None); // sparse or zeroed
}
```

---

## 4. Grain table directory offset is in sectors

`gd_offset` in the `SparseExtentHeader` is in **sectors**, not bytes:

```rust
file.seek(SeekFrom::Start(hdr.gd_offset * SECTOR_SIZE))?;
```

The header is at byte 0 of the file. `gd_offset` is typically 4 (= byte 2048) for
the primary grain directory. Some VMDKs include a redundant grain directory (`rgd_offset`)
at a different location; our implementation uses only `gd_offset`.

---

## 5. Grain size must be a power of 2 in sectors

The spec requires grain size to be a power of 2 and at least 8 sectors. The default
`qemu-img` VMDK grain size is 128 sectors (64 KiB). Our implementation does not
explicitly validate the power-of-2 invariant, but divides by grain size — a non-power-of-2
would not cause a panic but would yield incorrect results at grain boundaries.

---

## 6. `compress_algorithm` field

Byte offset 77–78 in the header is the compression algorithm:

| Value | Meaning |
|-------|---------|
| 0 | No compression (raw sector data) |
| 1 | DEFLATE (stream-optimized VMDKs only) |

Our implementation rejects any non-zero value with `VmdkError::CompressedNotSupported`.
Stream-optimized VMDKs (created by `qemu-img -O vmdk -o subformat=streamOptimized`)
use compression=1 and have a fundamentally different on-disk layout that requires
a separate implementation path.

---

## 7. `num_gtes_per_gt` and division safety

`num_gtes_per_gt` is used as a divisor (`grain_idx / num_gtes_per_gt`). The spec
says it must be 512 for hosted VMDKs, but arbitrary values appear in the wild.
A value of 0 causes divide-by-zero. Validate at parse time:

```rust
if num_gtes_per_gt == 0 {
    return Err(VmdkError::InvalidGeometry("num_gtes_per_gt must be > 0".into()));
}
```

---

## 8. Embedded text descriptor — `createType` parsing

Every monolithic sparse VMDK embeds a NUL-terminated text descriptor inside the file,
starting at `descriptor_offset × 512` bytes and spanning `descriptor_size × 512` bytes.
The descriptor contains key=value pairs (including comment lines beginning with `#`).

```
# Disk DescriptorFile
version=1
CID=5e81b00f
parentCID=ffffffff
createType="monolithicSparse"

# Extent description
RW 2048 SPARSE "disk.vmdk"
...
```

**`createType` field** governs the VMDK subformat:

| Value | Meaning |
|-------|---------|
| `monolithicSparse` | Single-file sparse (supported) |
| `twoGbMaxExtentFlat` | Multi-extent flat (not supported) |
| `streamOptimized` | DEFLATE-compressed (not supported — version 3, rejected at header parse) |

**Parsing notes:**
- `descriptor_offset == 0` or `descriptor_size == 0` → no embedded descriptor; `disk_type()` returns `""`
- The text area is zero-padded after the descriptor text; scanning stops at the first `\0` byte
- `descriptor_size` is capped at 64 KiB for the read to guard against crafted images with enormous field values
- Line endings may be `\n` or `\r\n`; `str::lines()` handles both
- `createType` value is always ASCII and always double-quoted: `createType="monolithicSparse"`

**Empirically verified** against both corpus files:

| File | `descriptor_offset` | `descriptor_size` | `createType` |
|------|---------------------|-------------------|--------------|
| `minimal.vmdk` (QEMU 11.0.0) | 1 | 20 | `monolithicSparse` |
| `dfvfs_ext2.vmdk` (VMware4) | 1 | 20 | `monolithicSparse` |

---

## Upstream PR candidates

| Project | File | Suggested change |
|---------|------|-----------------|
| VMware VDF spec | §4.2 (GTE) | Explicitly document GTE value 1 as "zeroed grain, return zeros" with a note that values 0 and 1 must both be treated as sparse |
| QEMU | `block/vmdk.c` | Add comment at GTE decode explaining the 0/1 sparse cases with a reference to spec §4.2 |
