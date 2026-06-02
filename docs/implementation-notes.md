# VMDK Implementation Notes

Developer notes capturing format quirks, spec contradictions, and empirically verified
behaviour. Derived from byte-level analysis of the test corpus; authoritative source
is the *VMware Virtual Disk Format 1.1* spec (August 2011).

---

## 1. Magic, version, and rejection paths

All VMDKs with binary headers begin with the 4-byte little-endian magic:

```
4b 44 4d 56   →   0x564D_444B   (ASCII "KDMV" reversed)
```

Bytes 4–7 are the version (u32 LE). Versions 1 and 3 are accepted; all others
return `UnsupportedVersion(n)`.

**Empirically confirmed behaviour for all corpus files:**

| File | Size | Behaviour |
|------|------|-----------|
| `minimal.vmdk` | 65,536 B | Opens OK — monolithicSparse v1 |
| `dfvfs_ext2.vmdk` | 262,144 B | Opens OK — monolithicSparse v1 (VMware4) |
| `plaso_image.vmdk` | 131,072 B | Opens OK — monolithicSparse v1 (VMware Workstation 4) |
| `stream_opt.vmdk` | 65,536 B | Opens OK — streamOptimized v3 (bytes 4–7 = `03 00 00 00`, compress=1) |
| `flat.vmdk` | 344 B | `Io(UnexpectedEof)` via `open()` — too short for 512-byte header; `Ok` via `open_path()` |
| `flat-f001.vmdk` | 1,048,576 B | `BadMagic` — bytes 0–3 = `00 00 00 00` |
| `ms3-win.vmdk` | 1,024 B | `BadMagic` via `open()` (text file); `UnsupportedDiskType` via `open_path()` |

`flat.vmdk` is a pure text descriptor (starts with `# Disk DescriptorFile`). ASCII `#` = 0x23
would fail the magic check, but the 344-byte file fails earlier — `read_exact` into a 512-byte
buffer returns `UnexpectedEof` before parsing begins.

---

## 2. SparseExtentHeader — complete field map

The 512-byte header occupies byte 0 of every binary VMDK. All multi-byte values
are little-endian:

```
[0..4]   magic             u32  0x564D_444B
[4..8]   version           u32  1 or 3 (see §8)
[8..12]  flags             u32  (ignored)
[12..20] capacity          u64  virtual disk size in sectors
[20..28] grain_size        u64  grain size in sectors
[28..36] descriptor_offset u64  sector of embedded text descriptor
[36..44] descriptor_size   u64  length of descriptor area in sectors
[44..48] num_gtes_per_gt   u32  grain-table entries per grain table
[48..56] rgd_offset        u64  sector of redundant grain directory (ignored)
[56..64] gd_offset         u64  sector of primary grain directory  ← KEY
[64..72] overhead          u64  sectors before grain data begins
[77..79] compress_algorithm u16 0 = uncompressed (v1), 1 = DEFLATE (v3)
```

**Empirically verified values across corpus:**

| Field | `minimal.vmdk` | `dfvfs_ext2.vmdk` | `stream_opt.vmdk` | `plaso_image.vmdk` | `ms3-win disk-s001.vmdk` |
|-------|---------------|--------------------|-------------------|--------------------|-|
| `version` | 1 | 1 | 3 | 1 | 1 |
| `capacity` | 2,048 | 8,192 | 2,048 | 200 | 8,323,072 |
| `grain_size` | 128 | 128 | 128 | 128 | 128 |
| `num_gtes_per_gt` | 512 | 512 | 512 | 512 | 512 |
| `gd_offset` | 26 | 26 | 26 | 26 | 510 |
| `overhead` | 128 | 128 | 128 | 128 | n/a |
| `compress_algorithm` | 0 | 0 | 1 | 0 | 0 |

Note: `ms3-win disk-s001.vmdk` has `gd_offset=510` — not the canonical 26.
This is expected for `twoGbMaxExtentSparse` extents which embed their own metadata.

---

## 3. Two-level grain directory: GD → GT → GTE

Virtual offset resolution uses two levels of indirection:

```
Grain Directory (GD)   1 entry per grain table, at gd_offset×512
  └── Grain Table (GT) loaded on demand; 1 entry per grain (4 bytes each)
        └── GTE        u32 value; 0 or 1 = sparse; ≥ 2 = sector of grain data
```

Resolution arithmetic:

```rust
let grain_idx       = virtual_offset / grain_size_bytes;
let offset_in_grain = virtual_offset % grain_size_bytes;
let gd_idx          = grain_idx / num_gtes_per_gt;
let gte_idx         = grain_idx % num_gtes_per_gt;
let gt_sector       = grain_dir[gd_idx];                    // loaded at open
let gte_file_pos    = gt_sector as u64 * 512 + gte_idx * 4;
// read 4-byte GTE from gte_file_pos
let file_offset     = gte as u64 * 512 + offset_in_grain;
```

**Empirical verification** — `dfvfs_ext2.vmdk`:

```
gd_offset = 26  →  GD at byte 0x3400
  GD[0] = 27    →  GT at byte 0x3600
    GTE[0] = 128  →  grain 0 at byte 0x10000  (virtual bytes      0 –  65535)
    GTE[1] = 0    →  sparse                   (virtual bytes  65536 – 131071)
    GTE[2] = 256  →  grain 2 at byte 0x20000  (virtual bytes 131072 – 196607)
    GTE[3..63] = 0  → sparse
  GD[1..] = 0    →  all remaining grain tables unallocated
```

Ext2 superblock cross-check: the ext2 superblock lives at virtual byte 1024, which
falls in grain 0. The superblock magic (`0xEF53` LE) is at virtual byte 1080 = file
byte `0x10000 + 1080 = 0x10438`:

```
file byte 0x10438: 53 ef   →   0xEF53 (ext2 magic, LE) ✓
```

**pWnOS v2.0 (40 GiB, VMware Workstation 7):** GD at sector 5151, GT at sector 5161,
GTE[0] = 10368 — x86 MBR boot code (`eb 63 90`) confirmed at sector 10368.
Demonstrates the reader handles large disks with non-trivial GD placement.

### Grain directory is eagerly loaded; grain tables are on-demand

The GD is small (one 4-byte entry per GT, typically a few KB) and loaded into
`Vec<u32>` at `open()` time. GTs are read on demand per-access to avoid allocating
megabytes upfront for large disks.

---

## 4. GTE values 0 and 1: both mean "sparse, return zeros"

| GTE value | Meaning |
|-----------|---------|
| 0 | Not allocated / sparse — return zeros |
| 1 | Explicitly zeroed grain — return zeros |
| ≥ 2 | File sector offset of the grain data |

**Common pitfall:** treating only GTE = 0 as sparse. GTE = 1 is a valid "explicitly
zeroed" state used by some VMware tools; reading at file sector 1 (offset 512) would
yield wrong data.

```rust
if gte <= 1 {
    return Ok(None); // sparse or zeroed → caller fills zeros
}
```

---

## 5. `gd_offset` is in sectors, not bytes

`gd_offset` in the `SparseExtentHeader` is a **sector number**, not a byte offset:

```rust
reader.seek(SeekFrom::Start(hdr.gd_offset * SECTOR_SIZE))?;
```

The canonical QEMU/VMware metadata layout for monolithic sparse VMDKs:

```
sector  0       SparseExtentHeader (512 bytes)
sector  1–20    text descriptor (20 sectors, 10,240 bytes)
sector  21–25   redundant grain directory (ignored)
sector  26      primary grain directory  ← gd_offset
sector  27      grain table(s)
sector  128+    grain data               ← overhead
```

`twoGbMaxExtentSparse` extents use `gd_offset = 510` (verified on Metasploitable3
`disk-s001.vmdk`).

---

## 6. Grain size: power of 2, in sectors

The spec requires grain size to be a power of 2 and at least 8 sectors. All corpus
files use 128 sectors (64 KiB). `grain_size == 0` causes divide-by-zero and is caught
at parse time; non-power-of-2 values are not re-validated.

---

## 7. `num_gtes_per_gt` and division safety

`num_gtes_per_gt` is used as a divisor for `grain_idx / num_gtes_per_gt`. A value of
0 causes divide-by-zero; validated at parse time. All corpus files use 512.

---

## 8. Version 3 and `compress_algorithm`: streamOptimized

Byte offset 77–78 (u16 LE) is the compression algorithm:

| Value | Meaning |
|-------|---------|
| 0 | No compression — raw sector data |
| 1 | DEFLATE — used by streamOptimized (v3) |

The version/compress acceptance matrix:

| version | compress_algorithm | Result |
|---------|-------------------|--------|
| 1 | 0 | `Ok` — standard monolithicSparse |
| 3 | 1 | `Ok` — streamOptimized; `compressed = true` in parsed header |
| any | other combinations | `Err(CompressedNotSupported)` or `Err(UnsupportedVersion)` |

Implemented via:

```rust
match (version, compress_algorithm) {
    (VERSION, 0) | (VERSION_STREAM_OPT, 1) => {}
    _ => return Err(VmdkError::CompressedNotSupported),
}
```

`VERSION = 1`, `VERSION_STREAM_OPT = 3`.

**Key finding:** QEMU-generated empty streamOptimized disks (`qemu-img create -o
subformat=streamOptimized`) have an **identical GD/GT layout to v1** (gd_offset=26,
all GTEs=0). No DEFLATE decompression is needed — all grains are unmapped and
return zeros identically to a monolithicSparse empty disk.

Compressed streamOptimized disks (with actual grain data) are rejected at read time:

```rust
if compressed {
    return Err(io::Error::new(io::ErrorKind::Unsupported,
        "allocated compressed grains (streamOptimized) are not yet supported"));
}
```

---

## 9. Text descriptors: `createType` and extent parsing

### Text descriptor format (twoGbMaxExtentFlat, twoGbMaxExtentSparse, monolithicFlat)

Some VMDKs are text-only descriptor files (first byte `#`):

```
# Disk DescriptorFile
version=1
CID=<hex32>
parentCID=ffffffff
createType="<subformat>"

# Extent description
RW <sectors> FLAT "<filename>" <sector_offset>
RW <sectors> SPARSE "<filename>"
```

`open_path()` detects the `#` first byte and routes to `parse_text_descriptor()` in
`descriptor.rs`. Only `FLAT` extents are collected; `SPARSE` extents are ignored
(collected into `ExtentEntry` and excluded from `extents`).

**Reject-if-empty guard:** after parsing, if `extents` is empty but `create_type` is
non-empty, `open_path()` returns `Err(UnsupportedDiskType(create_type))` rather than
silently building a zero-byte virtual disk.

### Embedded text descriptor (monolithicSparse, streamOptimized)

Binary VMDKs embed a NUL-padded descriptor at `descriptor_offset × 512` bytes,
spanning `descriptor_size × 512` bytes. Both standard VMware and QEMU place it at
sector 1, allocated 20 sectors.

Only `createType` is extracted from embedded descriptors (stored as `disk_type`).
The FLAT extent map in embedded descriptors is not parsed — the binary GD/GT is used
for sparse VMDKs.

### Known `createType` values

| Value | Meaning | Support |
|-------|---------|---------|
| `monolithicSparse` | Single-file binary sparse | `open()` + `open_path()` |
| `streamOptimized` | Binary sparse, v3 header, DEFLATE grains | `open()` + `open_path()` (all-sparse only) |
| `twoGbMaxExtentFlat` | Text descriptor + raw extent files | `open_path()` only |
| `monolithicFlat` | Text descriptor + single raw extent | `open_path()` only |
| `twoGbMaxExtentSparse` | Text descriptor + binary sparse extents | `Err(UnsupportedDiskType)` |
| `vmfs` | VMware ESXi internal format | Not implemented |

---

## 10. Flat extents: MultiExtentReader

`twoGbMaxExtentFlat` VMDKs consist of a text descriptor referencing one or more raw
extent files. `MultiExtentReader` in `flat.rs` concatenates them into a single
`Read + Seek` stream:

```
descriptor (flat.vmdk):
  RW 2048 FLAT "flat-f001.vmdk" 0  →  2048 sectors at virtual byte 0

flat-f001.vmdk:  raw 1 MiB of zeros
```

Each `FlatExtent` stores `byte_start`, `byte_end`, `file_offset` (from the sector
offset field × 512), and a `BufReader<File>`. Reads scan the extent list for the one
covering the current position, seek to `file_offset + offset_in_extent`, and read.

The `VmdkFileReader = VmdkReader<Box<dyn ReadSeek + Send>>` type alias allows
`open_path()` to return a type-erased reader regardless of whether the inner stream is
a `File` (binary VMDK) or `MultiExtentReader` (flat VMDK).

---

## 11. twoGbMaxExtentSparse: layout and current rejection

Each extent file in a `twoGbMaxExtentSparse` VMDK has its own binary VMDK header
(magic `KDMV`, version 1, compress=0). The descriptor (`disk.vmdk`) is a text file
listing all extents as `SPARSE` entries.

**Metasploitable3 Windows 2008 (verified empirically):**

```
disk.vmdk:  createType="twoGbMaxExtentSparse"
            RW 8323072 SPARSE "disk-s001.vmdk"   (× 15 more extents)
            RW 983040  SPARSE "disk-s016.vmdk"

disk-s001.vmdk header:
  magic=0x564D444B  version=1  compress=0
  capacity=8323072  grain_size=128  gd_offset=510
```

Note `gd_offset=510` — not the canonical 26. Each extent has its own independent GD.

To support `twoGbMaxExtentSparse`, the reader would need to:
1. Parse SPARSE entries in the descriptor (currently ignored)
2. Open each extent file
3. For reads, determine which extent covers the virtual offset, then do GD/GT lookup
   within that extent

---

## Upstream PR candidates

| Project | File | Suggested change |
|---------|------|-----------------|
| VMware VDF spec | §4.2 (GTE) | Explicitly document GTE value 1 as "zeroed grain, return zeros"; note both 0 and 1 must be treated as sparse |
| QEMU | `block/vmdk.c` | Add comment at GTE decode explaining 0/1 sparse cases with spec §4.2 reference |
