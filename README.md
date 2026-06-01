[![Crates.io](https://img.shields.io/crates/v/vmdk.svg)](https://crates.io/crates/vmdk)
[![Docs.rs](https://img.shields.io/docsrs/vmdk)](https://docs.rs/vmdk)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![CI](https://github.com/SecurityRonin/vmdk/actions/workflows/ci.yml/badge.svg)](https://github.com/SecurityRonin/vmdk/actions/workflows/ci.yml)
[![Sponsor](https://img.shields.io/badge/sponsor-h4x0r-ea4aaa?logo=github-sponsors)](https://github.com/sponsors/h4x0r)

**Pure-Rust read-only VMware VMDK sparse disk image reader.**

Decodes monolithic sparse VMDK containers (VMware Workstation, Fusion, and ESXi-exported images)
and exposes a `Read + Seek` interface over the virtual sector stream. Navigates the two-level
grain directory / grain table structure to resolve virtual offsets to raw grain data.
Zero unsafe code, no C bindings.

```toml
[dependencies]
vmdk = "0.1"
```

---

## Usage

### Library

```rust
use vmdk::VmdkReader;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

let file = File::open("disk.vmdk")?;
let mut reader = VmdkReader::open(file)?;

println!("Disk type:         {}", reader.disk_type());        // "monolithicSparse"
println!("Virtual disk size: {}", reader.virtual_disk_size()); // bytes

// Read the first sector
let mut sector = [0u8; 512];
reader.read_exact(&mut sector)?;

// Seek anywhere — O(1) via two-level grain table
reader.seek(SeekFrom::Start(1_048_576))?;
```

`VmdkReader<R>` is generic over any `R: Read + Seek`, so it works with `File`,
`Cursor<Vec<u8>>`, or any custom reader:

```rust
// In-memory (tests, embedded use)
use std::io::Cursor;
let reader = VmdkReader::open(Cursor::new(bytes))?;

// Pass directly to a filesystem crate
let reader = VmdkReader::open(File::open("disk.vmdk")?)?;
// e.g. ext4::Filesystem::open(reader)?;
```

### CLI

The `vmdk-cli` crate ships a `vmdk` binary:

```
$ vmdk info disk.vmdk
File:              disk.vmdk
Format:            VMDK v1 (monolithicSparse)
Virtual disk size: 4,194,304 bytes (4.00 MiB)
Sector size:       512 bytes
```

Unsupported formats (stream-optimized, flat extents) print an error to stderr and
exit with a non-zero status.

---

## Supported formats

| Format | Status |
|--------|:------:|
| Monolithic sparse (`monolithicSparse`) | ✓ |
| VMware Workstation / Fusion native | ✓ |
| ESXi-exported sparse | ✓ |
| Flat extent (`twoGbMaxExtentFlat`, `monolithicFlat`) | — planned |
| Stream-optimised (`streamOptimized`) | — planned |

Read-only. Multi-extent flat and stream-optimised VMDKs are not yet supported;
`VmdkReader::open` returns `Err` (never panics) on unsupported inputs.

---

## Format overview

```
byte 0        SparseExtentHeader (512 bytes)
sector 1–20   embedded text descriptor (createType, CID, extent map)
sector 21–25  redundant grain directory (not used)
sector 26     primary grain directory — one u32 per grain table
sector 27+    grain tables — one u32 (sector offset) per grain
sector 128+   grain data — raw 64 KiB blocks of virtual sector content
```

Virtual offset resolution is O(1): one GD lookup (in-memory) + one GT read (4 bytes)
+ one grain data seek.

---

## Related crates

### Container readers

| Crate | Format | Notes |
|-------|--------|-------|
| [`ewf`](https://github.com/SecurityRonin/ewf) | E01 / EWF / Ex01 | Dominant professional forensic acquisition format |
| [`aff4`](https://github.com/SecurityRonin/aff4) | AFF4 v1 | Evimetry / aff4-imager forensic disk images with Map streams |
| [`vhdx`](https://github.com/SecurityRonin/vhdx) | Microsoft VHDX | Hyper-V, Windows 8+, WSL2, Azure disk container |
| [`vhd`](https://github.com/SecurityRonin/vhd) | Legacy VHD | Virtual PC / Hyper-V Generation-1 fixed and dynamic disk images |
| [`qcow2`](https://github.com/SecurityRonin/qcow2) | QCOW2 v2/v3 | QEMU / KVM / libvirt disk images |
| [`ufed`](https://github.com/SecurityRonin/ufed) | Cellebrite UFED | Physical mobile device dumps with UFD XML segment mapping |
| [`dd`](https://github.com/SecurityRonin/dd) | Raw / flat / gz | dd, dcfldd, and gzip-wrapped raw images |
| [`iso`](https://github.com/SecurityRonin/iso) | ISO 9660 | Optical disc images: multi-session, UDF bridge, Rock Ridge, Joliet, El Torito |
| [`dmg`](https://github.com/SecurityRonin/dmg) | Apple DMG / UDIF | macOS disk images with koly trailer, mish block tables, zlib decompression |
| [`dar`](https://github.com/SecurityRonin/dar) | DAR archive | Disk ARchiver archives with catalog index and CRC32 validation |

### Forensic analysers

| Crate | Format | Notes |
|-------|--------|-------|
| [`ewf-forensic`](https://github.com/SecurityRonin/ewf-forensic) | E01 | Structural integrity audit, Adler-32 / MD5 hash verification, and in-memory repair |
| [`vhdx-forensic`](https://github.com/SecurityRonin/vhdx-forensic) | VHDX | Forensic integrity analyser and in-memory repair tool for VHDX containers |

---

[Privacy Policy](https://securityronin.github.io/vmdk/privacy/) · [Terms of Service](https://securityronin.github.io/vmdk/terms/) · © 2026 Security Ronin Ltd
