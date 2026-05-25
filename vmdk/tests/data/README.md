# tests/data — VMDK Real-Image Corpus

Integration test fixtures and fuzz seed corpus.
`fuzz/corpus/fuzz_open/` symlinks here; files are not duplicated.

## Files

All images generated locally with `qemu-img 11.0.0` on macOS (Apple Silicon).

| File | Subformat | Virtual size | Supported | Notes |
|------|-----------|-------------|-----------|-------|
| `minimal.vmdk` | monolithicSparse (v1) | 1 MiB | Yes | Primary integration test seed |
| `stream_opt.vmdk` | streamOptimized (v3) | 1 MiB | No | Returns `UnsupportedVersion(3)` |
| `flat.vmdk` | twoGbMaxExtentFlat | 1 MiB | No | Text descriptor file; returns parse error |
| `flat-f001.vmdk` | (extent data for flat.vmdk) | — | No | Raw extent, not a VMDK header |

"Not supported" means `VmdkReader::open` returns `Err`, not that it panics.
These files serve as regression seeds: the reader must not panic on any of them.

## Regenerating

```sh
qemu-img create -f vmdk tests/data/minimal.vmdk 1M
qemu-img create -f vmdk -o subformat=streamOptimized tests/data/stream_opt.vmdk 1M
qemu-img create -f vmdk -o subformat=twoGbMaxExtentFlat tests/data/flat.vmdk 1M
```
