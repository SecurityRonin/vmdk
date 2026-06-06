//! Forensic integrity analysis for VMware VMDK images.

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use vmdk::testutil::test_sparse_vmdk;

    #[test]
    fn header_provenance_clean_image() {
        let v = test_sparse_vmdk(&[0u8; 512]);
        let mut a = VmdkIntegrity::new(Cursor::new(v));
        let p = a.header_provenance().expect("io").expect("VMDK4 header");
        assert_eq!(p.version, 1);
        assert!(!p.unclean_shutdown);
        assert!(p.newline_check_intact);
    }

    #[test]
    fn validate_rgd_true_on_healthy_image() {
        let v = test_sparse_vmdk(&[0xAB; 512]);
        let mut a = VmdkIntegrity::new(Cursor::new(v));
        assert!(a.validate_rgd().expect("io"));
    }

    #[test]
    fn validate_rgd_false_on_redundant_gt_divergence() {
        // Corrupt the redundant grain table (sector 22 in the test fixture).
        let mut v = test_sparse_vmdk(&[0xAB; 512]);
        v[22 * 512] ^= 0xFF;
        let mut a = VmdkIntegrity::new(Cursor::new(v));
        assert!(!a.validate_rgd().expect("io"));
    }

    #[test]
    fn grain_directory_recovery_flags_recoverable_damage() {
        let mut v = test_sparse_vmdk(&[0xAB; 512]);
        v[21 * 512..21 * 512 + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // primary GD[0] damaged
        let mut a = VmdkIntegrity::new(Cursor::new(v));
        let r = a.grain_directory_recovery().expect("io");
        assert!(r.has_rgd);
        assert_eq!(r.primary_damaged, 1);
        assert_eq!(r.recoverable_via_rgd, 1);
    }

    #[test]
    fn check_integrity_clean_then_flags_dangling_gt() {
        let v = test_sparse_vmdk(&[0xAB; 512]);
        let mut a = VmdkIntegrity::new(Cursor::new(v));
        assert!(a.check_integrity().expect("io").is_ok());

        let mut v2 = test_sparse_vmdk(&[0xAB; 512]);
        v2[21 * 512..21 * 512 + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        let mut a2 = VmdkIntegrity::new(Cursor::new(v2));
        let rep = a2.check_integrity().expect("io");
        assert!(!rep.is_ok());
        assert_eq!(rep.out_of_bounds_grain_tables, 1);
    }

    #[test]
    fn analyse_reports_rgd_mismatch_anomaly() {
        let mut v = test_sparse_vmdk(&[0xAB; 512]);
        v[22 * 512] ^= 0xFF; // redundant GT diverges
        let mut a = VmdkIntegrity::new(Cursor::new(v));
        let anomalies = a.analyse().expect("io");
        assert!(
            anomalies
                .iter()
                .any(|x| matches!(x.kind, AnomalyKind::RedundantGdMismatch)),
            "expected an RGD mismatch anomaly, got: {anomalies:?}"
        );
    }

    #[test]
    fn analyse_clean_image_has_no_error_anomalies() {
        let v = test_sparse_vmdk(&[0xAB; 512]);
        let mut a = VmdkIntegrity::new(Cursor::new(v));
        let anomalies = a.analyse().expect("io");
        assert!(anomalies.iter().all(|x| x.severity != Severity::Error));
    }
}
