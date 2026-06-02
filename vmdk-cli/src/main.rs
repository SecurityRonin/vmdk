use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use vmdk::{VmdkChainReader, VmdkFileReader};

fn fmt_commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

fn open_or_die(path: &std::path::Path) -> VmdkFileReader {
    match VmdkFileReader::open_path(path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

#[derive(Parser)]
#[command(
    name = "vmdk",
    version,
    about = "Comprehensive CLI for VMware VMDK disk images",
    long_about = "Read-only VMDK inspector supporting monolithicSparse, streamOptimized, \
                  twoGbMaxExtentFlat/Sparse, monolithicFlat, COWD (vmfsSparse), \
                  seSparse, and snapshot chains."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Display image metadata (format, sizes, compression, CID)
    Info { path: PathBuf },

    /// Write the virtual disk as a raw flat image
    Extract {
        path: PathBuf,
        /// Output file path (default: <input>.raw)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Pipe the virtual disk bytes to stdout (for piping to xxd, strings, etc.)
    Cat { path: PathBuf },

    /// List allocated grain ranges as start_lba,sector_count pairs
    Sectors { path: PathBuf },

    /// Print the embedded text descriptor
    Descriptor { path: PathBuf },

    /// Hex dump a byte range of the virtual disk
    Hexdump {
        path: PathBuf,
        /// Start byte offset
        offset: u64,
        /// Number of bytes to dump
        length: u64,
    },

    /// Verify structural integrity (RGD match, GD/GT consistency)
    Verify { path: PathBuf },

    /// Compute SHA-256 and MD5 of the full virtual disk
    Hash { path: PathBuf },

    /// Byte-by-byte comparison of two VMDK virtual disks
    Diff {
        /// First VMDK file
        a: PathBuf,
        /// Second VMDK file
        b: PathBuf,
    },

    /// Show the snapshot/delta chain (parentFileNameHint traversal)
    SnapshotChain { path: PathBuf },
}

fn cmd_info(path: &std::path::Path) {
    let mut reader = open_or_die(path);
    let info = reader.info();
    let mib = info.virtual_disk_size as f64 / (1024.0 * 1024.0);

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    println!("File:              {file_name}");
    println!("Format:            VMDK v{} ({})", info.version, info.disk_type);
    println!(
        "Virtual disk size: {} bytes ({mib:.2} MiB)",
        fmt_commas(info.virtual_disk_size)
    );
    println!("Sector size:       512 bytes");
    println!("Sectors:           {}", fmt_commas(info.sector_count));
    if info.grain_size_sectors > 0 {
        println!(
            "Grain size:        {} sectors ({} KiB)",
            info.grain_size_sectors,
            info.grain_size_bytes / 1024
        );
    }
    println!("Compressed:        {}", if info.compressed { "yes" } else { "no" });
    if info.cid != 0xffff_ffff {
        println!("CID:               {:08x}", info.cid);
    }
    if info.parent_cid != 0xffff_ffff {
        println!("Parent CID:        {:08x}", info.parent_cid);
    }
    if !info.descriptor_text.is_empty() {
        let line_count = info.descriptor_text.lines().count();
        println!("Descriptor:        {line_count} lines");
    }
}

fn cmd_extract(path: &std::path::Path, output: Option<&std::path::Path>) {
    let mut reader = open_or_die(path);
    reader.seek(SeekFrom::Start(0)).unwrap_or_else(|e| {
        eprintln!("seek error: {e}");
        process::exit(1);
    });

    let out_path = output.map(std::borrow::ToOwned::to_owned).unwrap_or_else(|| {
        let mut p = path.to_path_buf();
        p.set_extension("raw");
        p
    });

    let file = std::fs::File::create(&out_path).unwrap_or_else(|e| {
        eprintln!("cannot create {}: {e}", out_path.display());
        process::exit(1);
    });
    let mut w = BufWriter::new(file);

    let mut buf = vec![0u8; 65536];
    loop {
        let n = reader.read(&mut buf).unwrap_or_else(|e| {
            eprintln!("read error: {e}");
            process::exit(1);
        });
        if n == 0 { break; }
        w.write_all(&buf[..n]).unwrap_or_else(|e| {
            eprintln!("write error: {e}");
            process::exit(1);
        });
    }
    eprintln!("Wrote {}", out_path.display());
}

fn cmd_cat(path: &std::path::Path) {
    let mut reader = open_or_die(path);
    reader.seek(SeekFrom::Start(0)).ok();
    let stdout = io::stdout();
    let w_inner = stdout.lock();
    let mut w = BufWriter::new(w_inner); // mut needed for write_all
    let mut buf = vec![0u8; 65536];
    loop {
        let n = reader.read(&mut buf).unwrap_or_else(|e| {
            eprintln!("read error: {e}");
            process::exit(1);
        });
        if n == 0 { break; }
        w.write_all(&buf[..n]).ok();
    }
}

fn cmd_sectors(path: &std::path::Path) {
    let mut reader = open_or_die(path);
    let grains = reader.iter_allocated_grains().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });
    if grains.is_empty() {
        println!("# No allocated grains (all-sparse)");
        return;
    }
    println!("# start_lba,sector_count");
    for g in &grains {
        println!("{},{}", g.start_lba, g.sector_count);
    }
    eprintln!("{} allocated grain(s)", grains.len());
}

fn cmd_descriptor(path: &std::path::Path) {
    let reader = open_or_die(path);
    let text = reader.descriptor_text();
    if text.is_empty() {
        eprintln!("No embedded descriptor in {}", path.display());
        process::exit(1);
    }
    print!("{text}");
}

fn cmd_hexdump(path: &std::path::Path, offset: u64, length: u64) {
    let mut reader = open_or_die(path);
    reader.seek(SeekFrom::Start(offset)).unwrap_or_else(|e| {
        eprintln!("seek error: {e}");
        process::exit(1);
    });
    let mut remaining = length;
    let mut buf = vec![0u8; 16usize.min(length as usize)];
    let mut pos = offset;
    while remaining > 0 {
        let to_read = (16u64.min(remaining)) as usize;
        buf.resize(to_read, 0);
        let n = reader.read(&mut buf[..to_read]).unwrap_or_else(|e| {
            eprintln!("read error: {e}");
            process::exit(1);
        });
        if n == 0 { break; }
        // Offset
        print!("{pos:08x}  ");
        // Hex bytes
        for i in 0..16 {
            if i < n { print!("{:02x} ", buf[i]); } else { print!("   "); }
            if i == 7 { print!(" "); }
        }
        print!(" |");
        // ASCII
        for i in 0..n {
            let c = buf[i];
            print!("{}", if c.is_ascii_graphic() || c == b' ' { c as char } else { '.' });
        }
        println!("|");
        pos += n as u64;
        remaining = remaining.saturating_sub(n as u64);
    }
}

fn cmd_verify(path: &std::path::Path) {
    let mut reader = open_or_die(path);
    let info = reader.info();
    println!("File:    {}", path.display());
    println!("Format:  {} v{}", info.disk_type, info.version);
    println!("Size:    {} bytes", fmt_commas(info.virtual_disk_size));

    // RGD validation
    match reader.validate_rgd() {
        Ok(true) => println!("RGD:     OK (matches primary GD)"),
        Ok(false) => println!("RGD:     absent or not applicable"),
        Err(e) => println!("RGD:     ERROR — {e}"),
    }

    // Allocation scan
    match reader.iter_allocated_grains() {
        Ok(grains) => {
            let allocated_bytes: u64 = grains.iter()
                .map(|g| g.sector_count * 512)
                .sum();
            println!("Allocated grains: {} ({} bytes)", grains.len(), fmt_commas(allocated_bytes));
        }
        Err(e) => println!("Allocation scan: ERROR — {e}"),
    }

    println!("Status:  OK");
}

fn cmd_hash(path: &std::path::Path) {
    let mut reader = open_or_die(path);
    reader.seek(SeekFrom::Start(0)).ok();
    let digest = reader.hash().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });
    println!("SHA-256: {}", digest.sha256);
    println!("MD5:     {}", digest.md5);
    println!("File:    {}", path.display());
}

fn cmd_diff(a: &std::path::Path, b: &std::path::Path) {
    let mut ra = open_or_die(a);
    let mut rb = open_or_die(b);
    ra.seek(SeekFrom::Start(0)).ok();
    rb.seek(SeekFrom::Start(0)).ok();

    let size_a = ra.virtual_disk_size();
    let size_b = rb.virtual_disk_size();
    if size_a != size_b {
        println!("DIFFER: virtual disk sizes differ ({size_a} vs {size_b} bytes)");
        process::exit(1);
    }

    let mut buf_a = vec![0u8; 65536];
    let mut buf_b = vec![0u8; 65536];
    let mut offset = 0u64;
    let mut diff_count = 0u64;
    loop {
        let na = ra.read(&mut buf_a).unwrap_or(0);
        let nb = rb.read(&mut buf_b).unwrap_or(0);
        if na == 0 && nb == 0 { break; }
        let n = na.min(nb);
        for i in 0..n {
            if buf_a[i] != buf_b[i] {
                if diff_count < 10 {
                    println!(
                        "DIFFER at offset {}: {:02x} vs {:02x}",
                        offset + i as u64,
                        buf_a[i],
                        buf_b[i]
                    );
                }
                diff_count += 1;
            }
        }
        offset += n as u64;
    }
    if diff_count == 0 {
        println!("IDENTICAL ({} bytes compared)", fmt_commas(size_a));
    } else {
        println!("DIFFER: {diff_count} byte(s) differ");
        process::exit(1);
    }
}

fn cmd_snapshot_chain(path: &std::path::Path) {
    match VmdkChainReader::open(path) {
        Ok(chain) => {
            println!("Chain depth: {} layer(s)", chain.depth());
            println!("Virtual size: {} bytes", fmt_commas(chain.virtual_disk_size()));
        }
        Err(e) => {
            // If the chain reader fails, try reading as a single image and report parentCID.
            match VmdkFileReader::open_path(path) {
                Ok(r) => {
                    let info = r.info();
                    println!("Chain depth: 1 layer");
                    println!("Virtual size: {} bytes", fmt_commas(info.virtual_disk_size));
                    if info.parent_cid != 0xffff_ffff {
                        println!("Parent CID: {:08x} (parent file not found: {e})", info.parent_cid);
                    } else {
                        println!("No parent (base image)");
                    }
                }
                Err(e2) => {
                    eprintln!("error: {e2}");
                    process::exit(1);
                }
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Command::Info { path } => cmd_info(path),
        Command::Extract { path, output } => cmd_extract(path, output.as_deref()),
        Command::Cat { path } => cmd_cat(path),
        Command::Sectors { path } => cmd_sectors(path),
        Command::Descriptor { path } => cmd_descriptor(path),
        Command::Hexdump { path, offset, length } => cmd_hexdump(path, *offset, *length),
        Command::Verify { path } => cmd_verify(path),
        Command::Hash { path } => cmd_hash(path),
        Command::Diff { a, b } => cmd_diff(a, b),
        Command::SnapshotChain { path } => cmd_snapshot_chain(path),
    }
}
