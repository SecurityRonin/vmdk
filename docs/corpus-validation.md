# VMDK Corpus Validation

Byte-level differential tests comparing `VmdkReader` output against
`qemu-img convert -O raw` (QEMU 11.0.0, macOS/Apple Silicon).

## Test Environment

| Component | Version |
|-----------|---------|
| QEMU | 11.0.0 (Homebrew, `/opt/homebrew/bin/qemu-img`) |
| OS | macOS (Apple Silicon) |
| Rust | (see `rust-toolchain.toml`) |

## Corpus Files

### dfvfs_ext2.vmdk — third-party validation target (VMware4 origin)

| Field | Value |
|-------|-------|
| Subformat | `monolithicSparse`, `virtualHWVersion = "4"` (VMware4 format) |
| Virtual size | 4 MiB (4,194,304 bytes) |
| Source | log2timeline/dfvfs test corpus (Apache-2.0) |
| URL | https://github.com/log2timeline/dfvfs/raw/main/test_data/ext2.vmdk |
| SHA-256 | `578b5f75af790030113a92c4227c6e53dad53a17e65cb491781dc75b3cef31f8` |
| Creator | VMware (confirmed by `file` output: "VMware4 disk image"; descriptor `ddb.virtualHWVersion = "4"`) |

**This is NOT QEMU-generated.** It was created by VMware, providing genuine
cross-implementation validation — a real VMware-format image read by our parser.

### minimal.vmdk — QEMU-generated reference

| Field | Value |
|-------|-------|
| Subformat | `monolithicSparse` (v1, extent descriptor version 1) |
| Virtual size | 1 MiB (1,048,576 bytes) |
| Creator | `qemu-img create -f vmdk vmdk/tests/data/minimal.vmdk 1M` (QEMU 11.0.0) |
| License | Generated locally — no external source |

Used for the synthetic-data differential test (same QEMU for write and verify).

### Unsupported format variants (regression seeds — no byte comparison)

| File | Subformat | Expected behaviour |
|------|-----------|--------------------|
| `stream_opt.vmdk` | `streamOptimized` (v3) | `Err(UnsupportedVersion(3))` |
| `flat.vmdk` | `twoGbMaxExtentFlat` | `Err(...)` (text descriptor) |
| `flat-f001.vmdk` | raw extent data | `Err(...)` (no VMDK header) |

These exist to verify `VmdkReader::open` returns `Err`, not panics, on
formats outside the implementation scope.

## Test Results

### `corpus_dfvfs_ext2_vmdk_reads_match_qemu_raw_convert` (VMware4, independent)

Full stride scan (4 KiB step) of `dfvfs_ext2.vmdk` — a real VMware4 image not
created by QEMU — compared against `qemu-img convert -O raw`. **PASS**.

Exercises: GD/GT lookup, grain reads, and format fields written by VMware
rather than QEMU (independently validates our descriptor parser).

### `vmdk_reads_match_qemu_raw_convert` (synthetic)

Synthetic 1 MiB sparse VMDK written by the test helper; compared byte-for-byte
against `qemu-img convert -O raw`. **PASS**.

### `corpus_minimal_vmdk_reads_match_qemu_raw_convert` (real image)

Full byte scan of `minimal.vmdk` at 64 KiB stride + near-end read, compared
against `qemu-img convert -O raw`. **PASS**.

Exercises: grain directory (GD) + grain table (GT) lookup, sparse grain
detection (GTE = 0 → return zeros), grain data reads.

## Validation Coverage

| Feature | Covered | Notes |
|---------|---------|-------|
| monolithicSparse v1 | Yes | `minimal.vmdk` + `dfvfs_ext2.vmdk` |
| VMware4 format (virtualHWVersion=4) | Yes | `dfvfs_ext2.vmdk` (third-party) |
| Sparse grains (GTE = 0) | Yes | unwritten regions of minimal.vmdk |
| Allocated grains | Yes | minimal.vmdk + dfvfs_ext2.vmdk |
| streamOptimized (v3) | Negative only | `stream_opt.vmdk` returns Err |
| Flat / raw extent | Negative only | `flat.vmdk` returns Err |
| Compressed grains | No | streamOptimized only; out of scope |
| Split extent (2 GiB) | No | Not in current corpus |

## Reproducing

```sh
# Regenerate corpus
qemu-img create -f vmdk vmdk/tests/data/minimal.vmdk 1M
qemu-img create -f vmdk -o subformat=streamOptimized vmdk/tests/data/stream_opt.vmdk 1M
qemu-img create -f vmdk -o subformat=twoGbMaxExtentFlat vmdk/tests/data/flat.vmdk 1M

# Run validation tests
cargo test
```
