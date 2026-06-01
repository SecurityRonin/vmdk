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

Bytes 4–7 are the version (u32 LE). Only version 1 is supported.

**Empirically confirmed rejection paths** for all five corpus files:

| File | Size | Rejection point | Error |
|------|------|-----------------|-------|
| `minimal.vmdk` | 65,536 B | — | (supported) |
| `dfvfs_ext2.vmdk` | 262,144 B | — | (supported) |
| `stream_opt.vmdk` | 65,536 B | bytes 4–7 = `03 00 00 00` (version 3) | `UnsupportedVersion(3)` |
| `flat.vmdk` | 344 B | file too short to fill 512-byte header buffer | `Io(UnexpectedEof)` |
| `flat-f001.vmdk` | 1,048,576 B | bytes 0–3 = `00 00 00 00` ≠ magic | `BadMagic` |

`flat.vmdk` is a pure text descriptor (starts with `# Disk DescriptorFile`). Although
ASCII `#` = 0x23 would also fail the magic check, the 344-byte file fails earlier —
`read_exact` into a 512-byte buffer returns `UnexpectedEof` before parsing begins.

`flat-f001.vmdk` is the raw extent data file for `flat.vmdk`. Its first 4 bytes are all
zeros — completely valid file size, immediately rejected on magic mismatch.

---

## 2. SparseExtentHeader — complete field map

The 512-byte header occupies byte 0 of every monolithic VMDK. All multi-byte values
are little-endian:

```
[0..4]   magic             u32  0x564D_444B
[4..8]   version           u32  1 (only supported value)
[8..12]  flags             u32  (ignored by this implementation)
[12..20] capacity          u64  virtual disk size in sectors
[20..28] grain_size        u64  grain size in sectors
[28..36] descriptor_offset u64  sector of embedded text descriptor
[36..44] descriptor_size   u64  length of descriptor area in sectors
[44..48] num_gtes_per_gt   u32  grain-table entries per grain table
[48..56] rgd_offset        u64  sector of redundant grain directory (not used)
[56..64] gd_offset         u64  sector of primary grain directory  ← KEY
[64..72] overhead          u64  sectors before grain data begins
[77..79] compress_algorithm u16 0 = uncompressed, 1 = DEFLATE (rejected)
```

`compress_algorithm` is read and validated — any non-zero value returns
`VmdkError::CompressedNotSupported` — but is not stored in the parsed struct after
validation; it is a local variable that is dropped.

**Empirically verified** values from the two supported corpus files:

| Field | `minimal.vmdk` | `dfvfs_ext2.vmdk` |
|-------|---------------|-------------------|
| `capacity` | 2,048 (1 MiB) | 8,192 (4 MiB) |
| `grain_size` | 128 (64 KiB) | 128 (64 KiB) |
| `descriptor_offset` | 1 | 1 |
| `descriptor_size` | 20 | 20 |
| `num_gtes_per_gt` | 512 | 512 |
| `rgd_offset` | 21 | 21 |
| `gd_offset` | 26 | 26 |
| `overhead` | 128 | 128 |
| `compress_algorithm` | 0 | 0 |

Both files were produced by different tools (QEMU 11.0.0 vs VMware) and show
identical geometry choices — `gd_offset=26` is the QEMU/VMware canonical layout for
files up to `overhead-1 = 127` allocated sectors of metadata.

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
falls in grain 0 (virtual bytes 0–65535). The superblock magic (`0xEF53` LE) is at
superblock offset 56, i.e. virtual byte 1080 = file byte `0x10000 + 1080 = 0x10438`:

```
file byte 0x10438: 53 ef   →   0xEF53 (ext2 magic, LE) ✓
```

### Grain directory is eagerly loaded; grain tables are on-demand

The grain directory is small (one 4-byte entry per grain table, typically a few KB)
and is loaded entirely into `Vec<u32>` at `open()` time. Grain tables are read on
demand per read to avoid allocating megabytes of tables upfront.

---

## 4. GTE values 0 and 1: both mean "sparse, return zeros"

From the spec:

| GTE value | Meaning |
|-----------|---------|
| 0 | Not allocated / sparse — return zeros |
| 1 | Explicitly zeroed grain — return zeros |
| ≥ 2 | File sector offset of the grain data |

**Common pitfall:** treating only GTE = 0 as sparse. GTE = 1 is a valid "explicitly
zeroed" state used by some VMware tools; reading at sector 1 (file offset 512) would
yield wrong data.

```rust
if gte <= 1 {
    return Ok(None); // sparse or zeroed → caller fills zeros
}
```

**Empirically confirmed:** `dfvfs_ext2.vmdk` GTE[1] = 0 (virtual grain 1 is sparse).
`minimal.vmdk` — QEMU-created empty disk — has all GTEs set to 0; every read returns
zeros.

---

## 5. `gd_offset` is in sectors, not bytes

`gd_offset` in the `SparseExtentHeader` is a **sector number**, not a byte offset:

```rust
reader.seek(SeekFrom::Start(hdr.gd_offset * SECTOR_SIZE))?;
```

Both corpus files have `gd_offset = 26` → byte offset `26 × 512 = 0x3400`.
The RGD (`rgd_offset = 21` → byte `0x2A00`) is ignored; this implementation uses
only the primary GD at `gd_offset`.

The canonical QEMU/VMware metadata layout for a monolithic sparse VMDK:

```
sector  0       SparseExtentHeader (512 bytes)
sector  1–20    text descriptor (20 sectors, 10 240 bytes)
sector  21–25   redundant grain directory (5 sectors, ignored)
sector  26      primary grain directory  ← gd_offset
sector  27      grain table(s)
sector  128+    grain data               ← overhead
```

---

## 6. Grain size: power of 2, in sectors

The spec requires grain size to be a power of 2 and at least 8 sectors. Both QEMU
and VMware use 128 sectors (64 KiB) for 1–4 MiB virtual disks.

Our implementation divides by `grain_size` in the resolution arithmetic; `grain_size == 0`
causes divide-by-zero and is caught at parse time:

```rust
if grain_size == 0 {
    return Err(VmdkError::InvalidGeometry("grain_size must be > 0".into()));
}
```

A non-power-of-2 grain size would not panic but would misplace data at grain
boundaries; the spec invariant is trusted rather than re-validated.

---

## 7. `num_gtes_per_gt` and division safety

`num_gtes_per_gt` is used as a divisor for `grain_idx / num_gtes_per_gt`. The spec
says 512 for hosted VMDKs, but arbitrary values appear in crafted images. A value of
0 causes divide-by-zero; validated at parse time:

```rust
if num_gtes_per_gt == 0 {
    return Err(VmdkError::InvalidGeometry("num_gtes_per_gt must be > 0".into()));
}
```

Both corpus files use `num_gtes_per_gt = 512`.

---

## 8. `compress_algorithm` and stream-optimized VMDKs

Byte offset 77–78 (u16 LE) is the compression algorithm:

| Value | Meaning |
|-------|---------|
| 0 | No compression — raw sector data (supported) |
| 1 | DEFLATE — used by stream-optimized VMDKs only (rejected) |

`stream_opt.vmdk` (QEMU `subformat=streamOptimized`) has **both** `version = 3` and
`compress_algorithm = 1`. The version check fires first, returning
`UnsupportedVersion(3)` before the compression field is ever checked.

After the compression check the value is dropped — `compress_algorithm` is a local
variable in `SparseExtentHeader::parse`, not a stored field on the struct.

---

## 9. Embedded text descriptor — `createType` parsing

Every monolithic sparse VMDK embeds a NUL-padded text descriptor starting at
`descriptor_offset × 512` bytes, spanning `descriptor_size × 512` bytes. Both corpus
VMDKs locate it at **byte 512** (sector 1), allocated 10,240 bytes (20 sectors), with
the text occupying the first ~200–330 bytes and the rest zero-padded.

Descriptor text format (LF or CRLF terminated lines):

```
# Disk DescriptorFile
version=1
CID=<hex32>
parentCID=ffffffff
createType="<subformat>"

# Extent description
RW <sectors> SPARSE "<filename>"
...
```

**`createType`** identifies the VMDK subformat, double-quoted, on its own line:

| Value | Meaning |
|-------|---------|
| `monolithicSparse` | Single-file sparse with grain directory (supported) |
| `twoGbMaxExtentFlat` | Multi-file flat extents (not supported) |
| `streamOptimized` | DEFLATE-compressed grains (not supported) |

Parsing strategy:
1. Skip if `descriptor_offset == 0` or `descriptor_size == 0` — no embedded descriptor.
2. Read `min(descriptor_size × 512, 65536)` bytes (64 KiB cap guards against crafted
   images with huge `descriptor_size` values).
3. Truncate at first `\0` byte.
4. Scan lines for `createType=`; strip surrounding double-quotes.
5. Return empty string if not found.

**Empirically verified**:

```
minimal.vmdk    byte 0x200 (sector 1): createType="monolithicSparse"  CID=5e81b00f
dfvfs_ext2.vmdk byte 0x200 (sector 1): createType="monolithicSparse"  CID=dc80b6c7
```

---

## 10. Validated corpus

| File | Size | Source | `createType` | virtual size | Outcome |
|------|------|--------|-------------|-------------|---------|
| `minimal.vmdk` | 64 KiB | `qemu-img create -f vmdk … 1M` (QEMU 11.0.0) | `monolithicSparse` | 1 MiB | Opens; all grains sparse → reads return zeros |
| `dfvfs_ext2.vmdk` | 256 KiB | log2timeline/dfvfs corpus (Apache-2.0; VMware4 origin) | `monolithicSparse` | 4 MiB | Opens; GTE[0]=128, [1]=0, [2]=256; ext2 superblock at virtual byte 1080 confirmed |
| `stream_opt.vmdk` | 64 KiB | `qemu-img create -f vmdk -o subformat=streamOptimized … 1M` | `streamOptimized` | — | `UnsupportedVersion(3)` |
| `flat.vmdk` | 344 B | `qemu-img create -f vmdk -o subformat=twoGbMaxExtentFlat … 1M` (descriptor only) | `twoGbMaxExtentFlat` | — | `Io(UnexpectedEof)` — too short to fill header buffer |
| `flat-f001.vmdk` | 1 MiB | extent data for `flat.vmdk` | — | — | `BadMagic` — first 4 bytes `00 00 00 00` |

**`dfvfs_ext2.vmdk` byte-arithmetic trace** (independent non-QEMU validation):

```
header:  gd_offset=26 → GD at byte 0x3400
GD[0]=27             → GT at byte 0x3600
GT[0]=128 (GTE[0])   → grain 0 at byte 0x10000
GT[1]=0   (GTE[1])   → grain 1 sparse (returns zeros)
GT[2]=256 (GTE[2])   → grain 2 at byte 0x20000

ext2 superblock magic check:
  virtual byte 1080 = grain 0 offset 1080
  file byte 0x10000 + 1080 = 0x10438
  d[0x10438..0x1043A] = 53 ef → 0xEF53 (ext2 magic, LE) ✓
```

---

## Upstream PR candidates

| Project | File | Suggested change |
|---------|------|-----------------|
| VMware VDF spec | §4.2 (GTE) | Explicitly document GTE value 1 as "zeroed grain, return zeros"; note that both 0 and 1 must be treated as sparse |
| QEMU | `block/vmdk.c` | Add comment at GTE decode explaining the 0/1 sparse cases with a spec §4.2 reference |
