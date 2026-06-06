# vmdk-forensic

[![Crates.io](https://img.shields.io/crates/v/vmdk-forensic.svg)](https://crates.io/crates/vmdk-forensic)
[![docs.rs](https://img.shields.io/docsrs/vmdk-forensic)](https://docs.rs/vmdk-forensic)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/SecurityRonin/vmdk/actions/workflows/ci.yml/badge.svg)](https://github.com/SecurityRonin/vmdk/actions)
[![Sponsor](https://img.shields.io/badge/sponsor-h4x0r-ea4aaa?logo=github-sponsors)](https://github.com/sponsors/h4x0r)

Forensic integrity analysis for VMware VMDK images. The evidence-grade layer on top of the [`vmdk`](https://crates.io/crates/vmdk) reader — it **reparses the raw structure** (so it works on images too damaged to open cleanly) and reports the redundant-grain-directory, dangling-pointer, recovery, and header-provenance findings that `qemu-img` and `libvmdk` discard.

## Quick start

```toml
[dependencies]
vmdk-forensic = "0.1"
```

```rust
use vmdk_forensic::{VmdkIntegrity, Severity};

let mut a = VmdkIntegrity::new(std::fs::File::open("disk.vmdk")?);

for anomaly in a.analyse()? {
    if anomaly.severity >= Severity::Warning {
        println!("[{:?}] {:?} — {}", anomaly.severity, anomaly.kind, anomaly.detail);
    }
}
# Ok::<(), std::io::Error>(())
```

## What it detects

`analyse()` aggregates every check into a severity-graded `Vec<VmdkAnomaly>` (sorted
worst-first); each finding carries its `kind` and a plain-language `detail` of its
forensic significance.

| Severity | `AnomalyKind` | Meaning |
|---|---|---|
| Error | `RedundantGdMismatch` | The redundant grain directory diverges from the primary — the grain tables they reference hold different contents (compared by **content**, not pointers, so healthy two-copy images don't false-positive) |
| Error | `DanglingGrainTable` | A grain-table pointer points beyond end-of-file (truncation or tampering) |
| Error | `DanglingGrain` | A grain pointer points beyond end-of-file |
| Error | `PrimaryGdUnrecoverable` | The primary grain directory is damaged with no RGD recovery available |
| Error | `FtpAsciiMangled` | Header newline-detection bytes were rewritten by an ASCII-mode FTP transfer |
| Warning | `PrimaryGdRecoverableViaRgd` | The primary grain directory is damaged but recoverable via the redundant copy |
| Warning | `UncleanShutdown` | `uncleanShutdown` flag set — the disk was not closed cleanly |

## Individual checks

Each finding is also available directly:

```rust
use vmdk_forensic::VmdkIntegrity;

let mut a = VmdkIntegrity::new(std::fs::File::open("disk.vmdk")?);

// Redundant-GD adjudication: are the grain tables the GD and RGD reference identical?
let rgd_ok = a.validate_rgd()?;

// Recovery triage: how much of a damaged primary GD can the RGD recover?
let rec = a.grain_directory_recovery()?;
println!("{} damaged, {} recoverable via RGD", rec.primary_damaged, rec.recoverable_via_rgd);

// Structural integrity: dangling GD/GT/grain pointers (VMDK4 sparse + seSparse).
let integ = a.check_integrity()?;
assert!(integ.is_ok());

// Header provenance: unclean-shutdown flag, FTP-ASCII-mangling, flag bits.
if let Some(p) = a.header_provenance()? {
    println!("unclean shutdown: {}", p.unclean_shutdown);
}
# Ok::<(), std::io::Error>(())
```

## Reader vs. analyzer

This is the same split as `vhdx`/`vhdx-forensic` and `ewf`/`ewf-forensic`:

- [`vmdk`](https://crates.io/crates/vmdk) — the lean `Read + Seek` reader. Use it to
  read virtual-disk bytes, including the opt-in RGD-fallback recovery read path.
- **`vmdk-forensic`** — this crate. Use it to audit an image before trusting it:
  tamper/corruption detection, recovery triage, and provenance. It re-exports
  `vmdk::VmdkReader`, so one dependency covers read + analysis.

## Security

Built to run on untrusted, potentially crafted images: every offset derived from a
header field uses saturating arithmetic and is bounds-checked before any read or
allocation; the grain-directory size is capped; zero `unsafe`.

---

[Privacy Policy](https://securityronin.github.io/vmdk/privacy/) · [Terms of Service](https://securityronin.github.io/vmdk/terms/) · © 2026 Security Ronin Ltd
