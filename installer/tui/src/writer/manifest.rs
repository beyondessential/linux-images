use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::progress::format_size;

// r[impl installer.write.source+2]
#[derive(Debug, Clone, Deserialize)]
pub struct PartitionManifest {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read only in tests; kept for manifest schema completeness"
        )
    )]
    pub arch: String,
    pub partitions: Vec<PartitionEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PartitionEntry {
    pub label: String,
    pub type_uuid: String,
    pub size_mib: u64,
    pub image: String,
}

// r[impl installer.write.source+2]
pub fn find_partition_manifest() -> Result<(PartitionManifest, PathBuf)> {
    let search_dirs = [
        "/run/live/medium/images",
        "/run/live/medium",
        "/cdrom/images",
        "/cdrom",
    ];

    for dir in &search_dirs {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }
        let manifest_path = dir_path.join("partitions.json");
        if manifest_path.is_file() {
            let contents = fs::read_to_string(&manifest_path)
                .with_context(|| format!("reading {}", manifest_path.display()))?;
            let manifest: PartitionManifest = serde_json::from_str(&contents)
                .with_context(|| format!("parsing {}", manifest_path.display()))?;
            return Ok((manifest, dir_path.to_path_buf()));
        }
    }

    bail!("no partitions.json found in search directories");
}

// r[impl installer.write.disk-size-check+2]
pub fn image_uncompressed_size(source: &Path) -> Result<u64> {
    let name = source
        .to_str()
        .with_context(|| format!("non-UTF-8 path: {}", source.display()))?;
    let base = name
        .strip_suffix(".zst")
        .with_context(|| format!("{} does not end in .zst", source.display()))?;
    let size_path_str = format!("{base}.size");
    let size_path = Path::new(&size_path_str);
    let contents = fs::read_to_string(size_path)
        .with_context(|| format!("reading size file {}", size_path.display()))?;
    contents.trim().parse::<u64>().with_context(|| {
        format!(
            "parsing size from {}: {:?}",
            size_path.display(),
            contents.trim()
        )
    })
}

// r[impl installer.write.disk-size-check+2]
pub fn partition_images_total_size(manifest: &PartitionManifest, images_dir: &Path) -> Result<u64> {
    let mut total: u64 = 0;
    for entry in &manifest.partitions {
        let img_path = images_dir.join(&entry.image);
        let size = image_uncompressed_size(&img_path)
            .with_context(|| format!("reading size for {}", entry.image))?;
        total += size;
    }
    Ok(total)
}

// r[impl installer.write.disk-size-check+2]
pub fn check_disk_size(image_size: u64, disk_size: u64) -> Result<()> {
    if disk_size < image_size {
        bail!(
            "target disk is too small: image requires {} but disk is only {}",
            format_size(image_size),
            format_size(disk_size),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.write.disk-size-check+2]
    #[test]
    fn check_disk_size_ok_when_equal() {
        assert!(check_disk_size(5 * 1024 * 1024 * 1024, 5 * 1024 * 1024 * 1024).is_ok());
    }

    // r[verify installer.write.disk-size-check+2]
    #[test]
    fn check_disk_size_ok_when_larger() {
        assert!(check_disk_size(5 * 1024 * 1024 * 1024, 10 * 1024 * 1024 * 1024).is_ok());
    }

    // r[verify installer.write.disk-size-check+2]
    #[test]
    fn check_disk_size_fails_when_too_small() {
        let result = check_disk_size(5 * 1024 * 1024 * 1024, 4 * 1024 * 1024 * 1024);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("too small"), "expected 'too small' in: {msg}");
        assert!(msg.contains("5.00 GiB"), "expected image size in: {msg}");
        assert!(msg.contains("4.00 GiB"), "expected disk size in: {msg}");
    }

    // r[verify installer.write.disk-size-check+2]
    #[test]
    fn image_uncompressed_size_reads_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let zst_path = dir.path().join("test.raw.zst");
        let size_path = dir.path().join("test.raw.size");

        std::fs::write(&zst_path, b"irrelevant").unwrap();
        std::fs::write(&size_path, "5368709120\n").unwrap();

        let size = image_uncompressed_size(&zst_path).unwrap();
        assert_eq!(size, 5_368_709_120);
    }

    // r[verify installer.write.disk-size-check+2]
    #[test]
    fn image_uncompressed_size_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let zst_path = dir.path().join("img.raw.zst");
        let size_path = dir.path().join("img.raw.size");

        std::fs::write(&zst_path, b"irrelevant").unwrap();
        std::fs::write(&size_path, "  1024  \n").unwrap();

        assert_eq!(image_uncompressed_size(&zst_path).unwrap(), 1024);
    }

    // r[verify installer.write.disk-size-check+2]
    #[test]
    fn image_uncompressed_size_fails_without_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let zst_path = dir.path().join("no-sidecar.raw.zst");
        std::fs::write(&zst_path, b"data").unwrap();

        assert!(image_uncompressed_size(&zst_path).is_err());
    }

    // r[verify installer.write.disk-size-check+2]
    #[test]
    fn image_uncompressed_size_fails_on_non_numeric() {
        let dir = tempfile::tempdir().unwrap();
        let zst_path = dir.path().join("bad.raw.zst");
        let size_path = dir.path().join("bad.raw.size");

        std::fs::write(&zst_path, b"data").unwrap();
        std::fs::write(&size_path, "not-a-number\n").unwrap();

        assert!(image_uncompressed_size(&zst_path).is_err());
    }

    // r[verify installer.write.disk-size-check+2]
    #[test]
    fn image_uncompressed_size_fails_without_zst_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.raw");

        assert!(image_uncompressed_size(&path).is_err());
    }

    // r[verify installer.write.disk-size-check+2]
    #[test]
    fn partition_images_total_size_sums_correctly() {
        let dir = tempfile::tempdir().unwrap();

        for (name, size_val) in [
            ("efi.img.zst", "536870912"),
            ("xboot.img.zst", "1073741824"),
            ("root.img.zst", "3758096384"),
        ] {
            std::fs::write(dir.path().join(name), b"data").unwrap();
            let size_name = name.replace(".zst", ".size");
            std::fs::write(dir.path().join(size_name), size_val).unwrap();
        }

        let manifest = PartitionManifest {
            arch: "amd64".into(),
            partitions: vec![
                PartitionEntry {
                    label: "efi".into(),
                    type_uuid: "C12A7328-F81F-11D2-BA4B-00A0C93EC93B".into(),
                    size_mib: 512,
                    image: "efi.img.zst".into(),
                },
                PartitionEntry {
                    label: "xboot".into(),
                    type_uuid: "BC13C2FF-59E6-4262-A352-B275FD6F7172".into(),
                    size_mib: 1024,
                    image: "xboot.img.zst".into(),
                },
                PartitionEntry {
                    label: "root".into(),
                    type_uuid: "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709".into(),
                    size_mib: 0,
                    image: "root.img.zst".into(),
                },
            ],
        };

        let total = partition_images_total_size(&manifest, dir.path()).unwrap();
        assert_eq!(total, 536870912 + 1073741824 + 3758096384);
    }

    // r[verify installer.write.source+2]
    #[test]
    fn parse_partition_manifest_valid() {
        let json = r#"{
            "arch": "amd64",
            "partitions": [
                {
                    "label": "efi",
                    "type_uuid": "C12A7328-F81F-11D2-BA4B-00A0C93EC93B",
                    "size_mib": 512,
                    "image": "efi.img.zst"
                },
                {
                    "label": "xboot",
                    "type_uuid": "BC13C2FF-59E6-4262-A352-B275FD6F7172",
                    "size_mib": 1024,
                    "image": "xboot.img.zst"
                },
                {
                    "label": "root",
                    "type_uuid": "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
                    "size_mib": 0,
                    "image": "root.img.zst"
                }
            ]
        }"#;

        let manifest: PartitionManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.arch, "amd64");
        assert_eq!(manifest.partitions.len(), 3);
        assert_eq!(manifest.partitions[0].label, "efi");
        assert_eq!(manifest.partitions[0].size_mib, 512);
        assert_eq!(manifest.partitions[1].label, "xboot");
        assert_eq!(manifest.partitions[1].size_mib, 1024);
        assert_eq!(manifest.partitions[2].label, "root");
        assert_eq!(manifest.partitions[2].size_mib, 0);
    }

    // r[verify installer.write.source+2]
    #[test]
    fn parse_partition_manifest_missing_fields() {
        let json = r#"{ "arch": "amd64" }"#;
        assert!(serde_json::from_str::<PartitionManifest>(json).is_err());
    }

    // r[verify installer.write.source+2]
    #[test]
    fn parse_partition_manifest_bad_json() {
        assert!(serde_json::from_str::<PartitionManifest>("not json").is_err());
    }
}
