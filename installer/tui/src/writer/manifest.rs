use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::progress::format_size;

// r[impl installer.write.source+5]
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

// r[impl installer.write.source+5]
pub fn find_partition_manifest() -> Result<(PartitionManifest, PathBuf)> {
    let search_dirs = [
        "/run/bes-images",
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

            // Warn if this is a legacy fallback path (no verity)
            if *dir != "/run/bes-images" {
                tracing::warn!(
                    "using fallback manifest at {} — integrity verification is NOT active",
                    manifest_path.display()
                );
            }

            return Ok((manifest, dir_path.to_path_buf()));
        }
    }

    bail!("no partitions.json found in search directories");
}

// r[impl installer.write.disk-size-check+3]
pub fn image_size(source: &Path) -> Result<u64> {
    let meta = fs::metadata(source).with_context(|| format!("stat {}", source.display()))?;
    Ok(meta.len())
}

// r[impl installer.write.disk-size-check+3]
pub fn partition_images_total_size(manifest: &PartitionManifest, images_dir: &Path) -> Result<u64> {
    let mut total: u64 = 0;
    for entry in &manifest.partitions {
        let img_path = images_dir.join(&entry.image);
        let size =
            image_size(&img_path).with_context(|| format!("reading size for {}", entry.image))?;
        total += size;
    }
    Ok(total)
}

// r[impl installer.write.disk-size-check+3]
/// Returns a list of `(filename, size)` pairs for all images in the manifest.
pub fn image_file_sizes(
    manifest: &PartitionManifest,
    images_dir: &Path,
) -> Result<Vec<(String, u64)>> {
    let mut result = Vec::with_capacity(manifest.partitions.len());
    for entry in &manifest.partitions {
        let img_path = images_dir.join(&entry.image);
        let size =
            image_size(&img_path).with_context(|| format!("reading size for {}", entry.image))?;
        result.push((entry.image.clone(), size));
    }
    Ok(result)
}

// r[impl installer.write.disk-size-check+3]
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

    // r[verify installer.write.disk-size-check+3]
    #[test]
    fn check_disk_size_ok_when_equal() {
        assert!(check_disk_size(5 * 1024 * 1024 * 1024, 5 * 1024 * 1024 * 1024).is_ok());
    }

    // r[verify installer.write.disk-size-check+3]
    #[test]
    fn check_disk_size_ok_when_larger() {
        assert!(check_disk_size(5 * 1024 * 1024 * 1024, 10 * 1024 * 1024 * 1024).is_ok());
    }

    // r[verify installer.write.disk-size-check+3]
    #[test]
    fn check_disk_size_fails_when_too_small() {
        let result = check_disk_size(5 * 1024 * 1024 * 1024, 4 * 1024 * 1024 * 1024);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("too small"), "expected 'too small' in: {msg}");
        assert!(msg.contains("5.00 GiB"), "expected image size in: {msg}");
        assert!(msg.contains("4.00 GiB"), "expected disk size in: {msg}");
    }

    // r[verify installer.write.disk-size-check+3]
    #[test]
    fn image_size_reads_file_length() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("test.img");

        let data = vec![0u8; 5_368_709];
        std::fs::write(&img_path, &data).unwrap();

        let size = image_size(&img_path).unwrap();
        assert_eq!(size, 5_368_709);
    }

    // r[verify installer.write.disk-size-check+3]
    #[test]
    fn image_size_fails_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nonexistent.img");

        assert!(image_size(&missing).is_err());
    }

    // r[verify installer.write.disk-size-check+3]
    #[test]
    fn partition_images_total_size_sums_correctly() {
        let dir = tempfile::tempdir().unwrap();

        // Create raw image files with known sizes
        std::fs::write(dir.path().join("efi.img"), vec![0u8; 536_870]).unwrap();
        std::fs::write(dir.path().join("xboot.img"), vec![0u8; 1_073_741]).unwrap();
        std::fs::write(dir.path().join("root.img"), vec![0u8; 3_758_096]).unwrap();

        let manifest = PartitionManifest {
            arch: "amd64".into(),
            partitions: vec![
                PartitionEntry {
                    label: "efi".into(),
                    type_uuid: "C12A7328-F81F-11D2-BA4B-00A0C93EC93B".into(),
                    size_mib: 512,
                    image: "efi.img".into(),
                },
                PartitionEntry {
                    label: "xboot".into(),
                    type_uuid: "BC13C2FF-59E6-4262-A352-B275FD6F7172".into(),
                    size_mib: 1024,
                    image: "xboot.img".into(),
                },
                PartitionEntry {
                    label: "root".into(),
                    type_uuid: "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709".into(),
                    size_mib: 0,
                    image: "root.img".into(),
                },
            ],
        };

        let total = partition_images_total_size(&manifest, dir.path()).unwrap();
        assert_eq!(total, 536_870 + 1_073_741 + 3_758_096);
    }

    // r[verify installer.write.disk-size-check+3]
    #[test]
    fn image_file_sizes_returns_pairs() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(dir.path().join("efi.img"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.path().join("root.img"), vec![0u8; 200]).unwrap();

        let manifest = PartitionManifest {
            arch: "amd64".into(),
            partitions: vec![
                PartitionEntry {
                    label: "efi".into(),
                    type_uuid: "C12A7328-F81F-11D2-BA4B-00A0C93EC93B".into(),
                    size_mib: 512,
                    image: "efi.img".into(),
                },
                PartitionEntry {
                    label: "root".into(),
                    type_uuid: "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709".into(),
                    size_mib: 0,
                    image: "root.img".into(),
                },
            ],
        };

        let sizes = image_file_sizes(&manifest, dir.path()).unwrap();
        assert_eq!(sizes.len(), 2);
        assert_eq!(sizes[0], ("efi.img".to_string(), 100));
        assert_eq!(sizes[1], ("root.img".to_string(), 200));
    }

    // r[verify installer.write.source+5]
    #[test]
    fn parse_partition_manifest_valid() {
        let json = r#"{
            "arch": "amd64",
            "partitions": [
                {
                    "label": "efi",
                    "type_uuid": "C12A7328-F81F-11D2-BA4B-00A0C93EC93B",
                    "size_mib": 512,
                    "image": "efi.img"
                },
                {
                    "label": "xboot",
                    "type_uuid": "BC13C2FF-59E6-4262-A352-B275FD6F7172",
                    "size_mib": 1024,
                    "image": "xboot.img"
                },
                {
                    "label": "root",
                    "type_uuid": "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
                    "size_mib": 0,
                    "image": "root.img"
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

    // r[verify installer.write.source+5]
    #[test]
    fn parse_partition_manifest_missing_fields() {
        let json = r#"{ "arch": "amd64" }"#;
        assert!(serde_json::from_str::<PartitionManifest>(json).is_err());
    }

    // r[verify installer.write.source+5]
    #[test]
    fn parse_partition_manifest_bad_json() {
        assert!(serde_json::from_str::<PartitionManifest>("not json").is_err());
    }
}
