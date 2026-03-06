use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

// r[impl installer.container.partition-devices+2]
pub fn ensure_partition_devices(target: &Path) -> Result<()> {
    let dev_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .context("getting device name from target path")?;

    let created = ensure_partition_devices_via_sysfs(dev_name)?;
    if created > 0 {
        tracing::info!("created {created} partition device node(s) via sysfs");
    } else {
        tracing::debug!("all partition device nodes already present for {dev_name}");
    }

    Ok(())
}

fn ensure_partition_devices_via_sysfs(dev_name: &str) -> Result<usize> {
    let sysfs_dir = format!("/sys/class/block/{dev_name}");
    let sysfs_path = Path::new(&sysfs_dir);
    if !sysfs_path.exists() {
        tracing::debug!("sysfs path {sysfs_dir} does not exist, skipping sysfs method");
        return Ok(0);
    }

    let entries =
        fs::read_dir(sysfs_path).with_context(|| format!("reading sysfs directory {sysfs_dir}"))?;

    let mut created = 0usize;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        let dev_file = entry.path().join("dev");
        if !dev_file.exists() {
            continue;
        }

        let majmin = fs::read_to_string(&dev_file)
            .with_context(|| format!("reading {}", dev_file.display()))?;
        let majmin = majmin.trim();
        let (major, minor) = parse_major_minor(majmin)?;

        let dev_path = Path::new("/dev").join(&*name);
        if is_valid_block_device(&dev_path, major, minor) {
            tracing::debug!("/dev/{name} already exists with correct major:minor {major}:{minor}");
            continue;
        }

        created += mknod_block_device(&dev_path, &name, major, minor)?;
    }

    Ok(created)
}

fn parse_major_minor(majmin: &str) -> Result<(u32, u32)> {
    let (major_str, minor_str) = majmin
        .split_once(':')
        .with_context(|| format!("parsing major:minor from {majmin:?}"))?;
    let major: u32 = major_str
        .parse()
        .with_context(|| format!("parsing major number from {major_str:?}"))?;
    let minor: u32 = minor_str
        .parse()
        .with_context(|| format!("parsing minor number from {minor_str:?}"))?;
    Ok((major, minor))
}

fn is_valid_block_device(path: &Path, expected_major: u32, expected_minor: u32) -> bool {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };

    if !meta.file_type().is_block_device() {
        tracing::debug!("{} exists but is not a block device", path.display());
        return false;
    }

    let rdev = meta.rdev();
    let actual_major = libc::major(rdev) as u32;
    let actual_minor = libc::minor(rdev) as u32;

    if actual_major == expected_major && actual_minor == expected_minor {
        return true;
    }

    tracing::debug!(
        "{} has major:minor {actual_major}:{actual_minor}, expected {expected_major}:{expected_minor}",
        path.display()
    );
    false
}

fn mknod_block_device(dev_path: &Path, name: &str, major: u32, minor: u32) -> Result<usize> {
    if dev_path.exists() {
        tracing::info!("removing stale /dev/{name} before recreating");
        let _ = fs::remove_file(dev_path);
    }

    tracing::info!("creating device node /dev/{name} (block {major}:{minor})");

    let status = Command::new("mknod")
        .args([
            dev_path.to_str().unwrap_or_default(),
            "b",
            &major.to_string(),
            &minor.to_string(),
        ])
        .output()
        .with_context(|| format!("running mknod for /dev/{name}"))?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        tracing::warn!("mknod /dev/{name} failed: {stderr}");
        return Ok(0);
    }

    Ok(1)
}

pub fn partition_path(device: &Path, part_num: u32) -> Result<PathBuf> {
    let dev_str = device.to_str().unwrap_or_default();

    let path = if dev_str.ends_with(|c: char| c.is_ascii_digit()) {
        PathBuf::from(format!("{dev_str}p{part_num}"))
    } else {
        PathBuf::from(format!("{dev_str}{part_num}"))
    };

    Ok(path)
}

pub fn reread_partition_table(target: &Path) -> Result<()> {
    let output = Command::new("partprobe")
        .arg(target)
        .output()
        .context("running partprobe")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("partprobe failed on {}: {stderr}", target.display());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        tracing::debug!("partprobe: {stderr}");
    }

    let _ = Command::new("udevadm")
        .args(["settle", "--timeout=5"])
        .status();

    ensure_partition_devices(target)?;

    Ok(())
}

pub(crate) fn run_command(program: &str, args: &[&str]) -> Result<()> {
    tracing::debug!("running: {program} {}", args.join(" "));

    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("spawning {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("{program} failed (exit {}): {stderr}", output.status);
        bail!("{program} failed (exit {}): {stderr}", output.status);
    }

    Ok(())
}

pub(crate) fn sync_device(file: &std::fs::File) -> Result<()> {
    file.sync_all().context("syncing target device")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.write.partitions+2]
    #[test]
    fn partition_path_scsi_disk() {
        let path = partition_path(Path::new("/dev/sda"), 3).unwrap();
        assert_eq!(path, PathBuf::from("/dev/sda3"));
    }

    // r[verify installer.write.partitions+2]
    #[test]
    fn partition_path_nvme() {
        let path = partition_path(Path::new("/dev/nvme0n1"), 3).unwrap();
        assert_eq!(path, PathBuf::from("/dev/nvme0n1p3"));
    }

    // r[verify installer.write.partitions+2]
    #[test]
    fn partition_path_loop() {
        let path = partition_path(Path::new("/dev/loop0"), 1).unwrap();
        assert_eq!(path, PathBuf::from("/dev/loop0p1"));
    }

    // r[verify installer.container.partition-devices+2]
    #[test]
    fn parse_major_minor_valid() {
        let (major, minor) = parse_major_minor("259:22").unwrap();
        assert_eq!(major, 259);
        assert_eq!(minor, 22);
    }

    // r[verify installer.container.partition-devices+2]
    #[test]
    fn parse_major_minor_zero() {
        let (major, minor) = parse_major_minor("0:0").unwrap();
        assert_eq!(major, 0);
        assert_eq!(minor, 0);
    }

    // r[verify installer.container.partition-devices+2]
    #[test]
    fn parse_major_minor_missing_colon() {
        assert!(parse_major_minor("259").is_err());
    }

    // r[verify installer.container.partition-devices+2]
    #[test]
    fn parse_major_minor_non_numeric() {
        assert!(parse_major_minor("abc:22").is_err());
        assert!(parse_major_minor("259:xyz").is_err());
    }

    // r[verify installer.container.partition-devices+2]
    #[test]
    fn parse_major_minor_empty() {
        assert!(parse_major_minor("").is_err());
    }

    // r[verify installer.container.partition-devices+2]
    #[test]
    fn is_valid_block_device_nonexistent_path() {
        assert!(!is_valid_block_device(
            Path::new("/dev/nonexistent_xyz_test"),
            8,
            0
        ));
    }

    // r[verify installer.container.partition-devices+2]
    #[test]
    fn is_valid_block_device_regular_file_is_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("regular_file");
        std::fs::write(&path, b"hello").unwrap();
        assert!(!is_valid_block_device(&path, 8, 0));
    }

    // r[verify installer.container.partition-devices+2]
    #[test]
    fn ensure_partition_devices_via_sysfs_nonexistent_dir() {
        let count = ensure_partition_devices_via_sysfs("nonexistent_device_xyzzy_test").unwrap();
        assert_eq!(count, 0);
    }
}
