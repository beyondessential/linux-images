use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::paths;
pub use crate::util::{partition_path, run_command};

// r[impl installer.container.partition-devices+3]
pub fn ensure_partition_devices(target: &Path) -> Result<()> {
    const MAX_RETRIES: u32 = 10;
    const RETRY_DELAY: Duration = Duration::from_millis(200);

    let dev_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .context("getting device name from target path")?;

    for attempt in 0..MAX_RETRIES {
        let (created, found) = ensure_partition_devices_via_sysfs(dev_name)?;
        if found > 0 {
            if created > 0 {
                tracing::info!("created {created} partition device node(s) via sysfs");
            } else {
                tracing::debug!(
                    "all {found} partition device nodes already present for {dev_name}"
                );
            }
            return Ok(());
        }

        if attempt + 1 < MAX_RETRIES {
            tracing::debug!(
                "no partition entries found in sysfs for {dev_name} (attempt {}/{}), retrying in {}ms",
                attempt + 1,
                MAX_RETRIES,
                RETRY_DELAY.as_millis(),
            );
            thread::sleep(RETRY_DELAY);
        }
    }

    tracing::warn!(
        "no partition sysfs entries appeared for {dev_name} after {} retries ({:.1}s) — \
         partition device nodes may be missing",
        MAX_RETRIES,
        (MAX_RETRIES as f64) * RETRY_DELAY.as_secs_f64(),
    );
    Ok(())
}

/// Returns `(created, found)` — the number of device nodes created and the
/// total number of partition entries discovered in sysfs.
fn ensure_partition_devices_via_sysfs(dev_name: &str) -> Result<(usize, usize)> {
    let sysfs_dir = format!("/sys/class/block/{dev_name}");
    let sysfs_path = Path::new(&sysfs_dir);
    if !sysfs_path.exists() {
        tracing::warn!("sysfs path {sysfs_dir} does not exist — cannot discover partitions");
        return Ok((0, 0));
    }

    let entries =
        fs::read_dir(sysfs_path).with_context(|| format!("reading sysfs directory {sysfs_dir}"))?;

    let mut seen_entries = Vec::new();
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

        seen_entries.push(format!("{name}({major}:{minor})"));

        let dev_path = Path::new("/dev").join(&*name);
        if is_valid_block_device(&dev_path, major, minor) {
            tracing::debug!("/dev/{name} already exists with correct major:minor {major}:{minor}");
            continue;
        }

        created += mknod_block_device(&dev_path, &name, major, minor)?;
    }

    let found = seen_entries.len();
    tracing::debug!(
        "ensure_partition_devices_via_sysfs({dev_name}): saw [{}], created {created}",
        seen_entries.join(", "),
    );

    Ok((created, found))
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

    let status = Command::new(paths::MKNOD)
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

pub fn reread_partition_table(target: &Path) -> Result<()> {
    let output = Command::new(paths::PARTPROBE)
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

    let _ = Command::new(paths::UDEVADM)
        .args(["settle", "--timeout=5"])
        .status();

    ensure_partition_devices(target)?;

    Ok(())
}

pub(crate) fn sync_device(file: &std::fs::File) -> Result<()> {
    file.sync_all().context("syncing target device")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.container.partition-devices+3]
    #[test]
    fn parse_major_minor_valid() {
        let (major, minor) = parse_major_minor("259:22").unwrap();
        assert_eq!(major, 259);
        assert_eq!(minor, 22);
    }

    // r[verify installer.container.partition-devices+3]
    #[test]
    fn parse_major_minor_zero() {
        let (major, minor) = parse_major_minor("0:0").unwrap();
        assert_eq!(major, 0);
        assert_eq!(minor, 0);
    }

    // r[verify installer.container.partition-devices+3]
    #[test]
    fn parse_major_minor_missing_colon() {
        assert!(parse_major_minor("259").is_err());
    }

    // r[verify installer.container.partition-devices+3]
    #[test]
    fn parse_major_minor_non_numeric() {
        assert!(parse_major_minor("abc:22").is_err());
        assert!(parse_major_minor("259:xyz").is_err());
    }

    // r[verify installer.container.partition-devices+3]
    #[test]
    fn parse_major_minor_empty() {
        assert!(parse_major_minor("").is_err());
    }

    // r[verify installer.container.partition-devices+3]
    #[test]
    fn is_valid_block_device_nonexistent_path() {
        assert!(!is_valid_block_device(
            Path::new("/dev/nonexistent_xyz_test"),
            8,
            0
        ));
    }

    // r[verify installer.container.partition-devices+3]
    #[test]
    fn is_valid_block_device_regular_file_is_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("regular_file");
        std::fs::write(&path, b"hello").unwrap();
        assert!(!is_valid_block_device(&path, 8, 0));
    }

    // r[verify installer.container.partition-devices+3]
    #[test]
    fn ensure_partition_devices_via_sysfs_nonexistent_dir() {
        let (created, found) =
            ensure_partition_devices_via_sysfs("nonexistent_device_xyzzy_test").unwrap();
        assert_eq!(created, 0);
        assert_eq!(found, 0);
    }
}
