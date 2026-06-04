//! Fuzz target: feed arbitrary bytes to VmdkReader::open.
//!
//! Invariant: must not panic; may return Ok or Err.
//!
//! Run with:
//!   cargo +nightly fuzz run fuzz_open
//!
//! Corpus seeds in fuzz/corpus/fuzz_open/ (add real VMDK files here for coverage).
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    // VmdkReader::open is generic over Read + Seek; a Cursor over the fuzz input
    // exercises the full header/descriptor parse without touching the filesystem.
    let _ = vmdk::VmdkReader::open(Cursor::new(data));
});
