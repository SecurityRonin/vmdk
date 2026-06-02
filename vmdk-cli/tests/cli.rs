use std::process::Command;

fn vmdk_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vmdk"))
}

fn data_path(name: &str) -> String {
    // CARGO_MANIFEST_DIR is vmdk-cli/ → workspace root is one level up
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("vmdk/tests/data")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn info_shows_virtual_disk_size_minimal() {
    let out = vmdk_bin()
        .args(["info", &data_path("minimal.vmdk")])
        .output()
        .expect("vmdk binary must run");
    assert!(out.status.success(), "exit status: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1,048,576") || stdout.contains("1 MiB"),
        "expected virtual disk size in output, got: {stdout}"
    );
}

#[test]
fn info_shows_format_monolithic_sparse() {
    let out = vmdk_bin()
        .args(["info", &data_path("minimal.vmdk")])
        .output()
        .expect("vmdk binary must run");
    assert!(out.status.success(), "exit status: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("monolithicSparse"),
        "expected monolithicSparse in format line, got: {stdout}"
    );
}

#[test]
fn info_shows_sector_size() {
    let out = vmdk_bin()
        .args(["info", &data_path("minimal.vmdk")])
        .output()
        .expect("vmdk binary must run");
    assert!(out.status.success(), "exit status: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("512"),
        "expected sector size 512 in output, got: {stdout}"
    );
}

#[test]
fn info_dfvfs_ext2_virtual_disk_size() {
    let out = vmdk_bin()
        .args(["info", &data_path("dfvfs_ext2.vmdk")])
        .output()
        .expect("vmdk binary must run");
    assert!(out.status.success(), "exit status: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("4,194,304") || stdout.contains("4 MiB"),
        "expected 4 MiB virtual disk size, got: {stdout}"
    );
}

#[test]
fn info_errors_on_missing_file() {
    let out = vmdk_bin()
        .args(["info", "nonexistent.vmdk"])
        .output()
        .expect("vmdk binary must run");
    assert!(
        !out.status.success(),
        "should exit non-zero for missing file"
    );
}

#[test]
fn info_shows_stream_optimized_disk_type() {
    let out = vmdk_bin()
        .args(["info", &data_path("stream_opt.vmdk")])
        .output()
        .expect("vmdk binary must run");
    assert!(
        out.status.success(),
        "stream_opt.vmdk info must succeed after v3 support, exit: {}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("streamOptimized"),
        "expected streamOptimized in output, got: {stdout}"
    );
}

#[test]
fn info_shows_flat_vmdk_disk_type() {
    let out = vmdk_bin()
        .args(["info", &data_path("flat.vmdk")])
        .output()
        .expect("vmdk binary must run");
    assert!(
        out.status.success(),
        "flat.vmdk info must succeed after open_path support, exit: {}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("twoGbMaxExtentFlat"),
        "expected twoGbMaxExtentFlat in output, got: {stdout}"
    );
}

// ── sectors command ───────────────────────────────────────────────────────────

#[test]
fn sectors_all_sparse_shows_no_allocated() {
    let out = vmdk_bin()
        .args(["sectors", &data_path("minimal.vmdk")])
        .output()
        .expect("vmdk sectors must run");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // minimal.vmdk is all-sparse; sectors header should appear
    assert!(
        stdout.contains("start_lba") || stdout.contains("No allocated") || stdout.contains("sparse"),
        "got: {stdout}"
    );
}

#[test]
fn sectors_dfvfs_shows_allocated_grains() {
    let out = vmdk_bin()
        .args(["sectors", &data_path("dfvfs_ext2.vmdk")])
        .output()
        .expect("vmdk sectors must run");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // dfvfs_ext2 has allocated grains — check for numeric output
    assert!(
        stdout.contains(',') || stdout.contains("allocated grain"),
        "expected allocated grain ranges, got: {stdout}"
    );
}

// ── descriptor command ────────────────────────────────────────────────────────

#[test]
fn descriptor_shows_create_type() {
    let out = vmdk_bin()
        .args(["descriptor", &data_path("minimal.vmdk")])
        .output()
        .expect("vmdk descriptor must run");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("createType") || stdout.contains("monolithicSparse"),
        "expected descriptor text, got: {stdout}"
    );
}

// ── hexdump command ───────────────────────────────────────────────────────────

#[test]
fn hexdump_outputs_hex_bytes() {
    let out = vmdk_bin()
        .args(["hexdump", &data_path("dfvfs_ext2.vmdk"), "0", "32"])
        .output()
        .expect("vmdk hexdump must run");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Output must contain hex digits and offset
    assert!(stdout.contains("00000000"), "expected hex offset, got: {stdout}");
}

// ── hash command ──────────────────────────────────────────────────────────────

#[test]
fn hash_produces_sha256_and_md5() {
    let out = vmdk_bin()
        .args(["hash", &data_path("minimal.vmdk")])
        .output()
        .expect("vmdk hash must run");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("SHA-256"), "expected SHA-256 line, got: {stdout}");
    assert!(stdout.contains("MD5"), "expected MD5 line, got: {stdout}");
}

#[test]
fn hash_minimal_vmdk_matches_known_md5() {
    let out = vmdk_bin()
        .args(["hash", &data_path("minimal.vmdk")])
        .output()
        .expect("vmdk hash must run");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Known MD5 from docs/validation.md
    assert!(
        stdout.contains("b6d81b360a5672d80c27430f39153e2c"),
        "MD5 mismatch, got: {stdout}"
    );
}

// ── verify command ────────────────────────────────────────────────────────────

#[test]
fn verify_minimal_vmdk_exits_ok() {
    let out = vmdk_bin()
        .args(["verify", &data_path("minimal.vmdk")])
        .output()
        .expect("vmdk verify must run");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("OK"), "expected OK in verify output, got: {stdout}");
}

// ── diff command ──────────────────────────────────────────────────────────────

#[test]
fn diff_identical_vmdk_reports_identical() {
    let path = data_path("minimal.vmdk");
    let out = vmdk_bin()
        .args(["diff", &path, &path])
        .output()
        .expect("vmdk diff must run");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("IDENTICAL"), "expected IDENTICAL, got: {stdout}");
}

#[test]
fn diff_different_vmdks_exits_nonzero() {
    let out = vmdk_bin()
        .args([
            "diff",
            &data_path("minimal.vmdk"),
            &data_path("dfvfs_ext2.vmdk"),
        ])
        .output()
        .expect("vmdk diff must run");
    assert!(
        !out.status.success(),
        "diff of different VMDKs (different sizes) must exit non-zero"
    );
}

// ── snapshot-chain command ────────────────────────────────────────────────────

#[test]
fn snapshot_chain_base_image_shows_depth_one() {
    let out = vmdk_bin()
        .args(["snapshot-chain", &data_path("minimal.vmdk")])
        .output()
        .expect("vmdk snapshot-chain must run");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1 layer") || stdout.contains("depth: 1"),
        "expected depth 1, got: {stdout}"
    );
}

// ── extract command ───────────────────────────────────────────────────────────

#[test]
fn extract_produces_raw_file_of_correct_size() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_path = tmp.path().join("out.raw");
    let status = vmdk_bin()
        .args([
            "extract",
            &data_path("minimal.vmdk"),
            "--output",
            out_path.to_str().unwrap(),
        ])
        .status()
        .expect("vmdk extract must run");
    assert!(status.success(), "exit: {status}");
    let meta = std::fs::metadata(&out_path).expect("output file must exist");
    assert_eq!(meta.len(), 1_048_576, "raw file must be 1 MiB");
}
