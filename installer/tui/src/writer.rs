use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};

// r[impl installer.tui.progress]
pub struct WriteProgress {
    pub bytes_written: u64,
    pub total_bytes: Option<u64>,
    pub elapsed: std::time::Duration,
}

impl WriteProgress {
    pub fn fraction(&self) -> Option<f64> {
        self.total_bytes
            .map(|total| self.bytes_written as f64 / total as f64)
    }

    pub fn eta(&self) -> Option<std::time::Duration> {
        let fraction = self.fraction()?;
        if fraction <= 0.0 {
            return None;
        }
        let total_estimated = self.elapsed.as_secs_f64() / fraction;
        let remaining = total_estimated - self.elapsed.as_secs_f64();
        if remaining < 0.0 {
            return Some(std::time::Duration::ZERO);
        }
        Some(std::time::Duration::from_secs_f64(remaining))
    }

    pub fn throughput_mbps(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        (self.bytes_written as f64) / (1024.0 * 1024.0) / secs
    }
}

pub fn format_eta(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

// r[impl installer.write.source]
pub fn find_image_path(variant: &str, arch: &str) -> Result<std::path::PathBuf> {
    let search_dirs = [
        "/run/live/medium/images",
        "/run/live/medium",
        "/cdrom/images",
        "/cdrom",
    ];

    let pattern = format!("{variant}-{arch}");

    for dir in &search_dirs {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }
        let entries =
            std::fs::read_dir(dir_path).with_context(|| format!("reading directory {dir}"))?;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.contains(&pattern) && name.ends_with(".raw.zst") {
                return Ok(entry.path());
            }
        }
    }

    bail!("no .raw.zst image found for variant={variant} arch={arch}");
}

// r[impl installer.write.disk-size-check]
/// Read the uncompressed image size from the `.size` sidecar file that the
/// build pipeline writes alongside the `.raw.zst`. The sidecar contains the
/// byte count as a decimal ASCII string (output of `stat --format='%s'`).
pub fn image_uncompressed_size(source: &Path) -> Result<u64> {
    let name = source
        .to_str()
        .with_context(|| format!("non-UTF-8 path: {}", source.display()))?;
    let base = name
        .strip_suffix(".zst")
        .with_context(|| format!("{} does not end in .zst", source.display()))?;
    let size_path_str = format!("{base}.size");
    let size_path = Path::new(&size_path_str);
    let contents = std::fs::read_to_string(size_path)
        .with_context(|| format!("reading size file {}", size_path.display()))?;
    contents.trim().parse::<u64>().with_context(|| {
        format!(
            "parsing size from {}: {:?}",
            size_path.display(),
            contents.trim()
        )
    })
}

// r[impl installer.write.disk-size-check]
/// Verify that the target disk is large enough for the uncompressed image.
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

/// Wipe all existing filesystem, RAID, and partition-table signatures from a disk.
// r[impl installer.write.partitions]
pub fn wipe_disk(target: &Path) -> Result<()> {
    tracing::info!("wiping existing signatures on {}", target.display());

    let wipefs_status = Command::new("wipefs")
        .args(["--all", "--force"])
        .arg(target)
        .output()
        .context("running wipefs")?;

    if !wipefs_status.status.success() {
        let stderr = String::from_utf8_lossy(&wipefs_status.stderr);
        tracing::warn!("wipefs failed (non-fatal): {stderr}");
    }

    let sgdisk_status = Command::new("sgdisk")
        .arg("--zap-all")
        .arg(target)
        .output()
        .context("running sgdisk --zap-all")?;

    if !sgdisk_status.status.success() {
        let stderr = String::from_utf8_lossy(&sgdisk_status.stderr);
        tracing::warn!("sgdisk --zap-all failed (non-fatal): {stderr}");
    }

    // Zero out the first and last 1 MiB to destroy any remaining MBR, GPT backup,
    // or LUKS headers that wipefs/sgdisk may have missed.
    if let Ok(mut f) = OpenOptions::new().write(true).open(target) {
        let zeros = vec![0u8; 1024 * 1024];
        let _ = f.write_all(&zeros);

        if let Ok(size) = std::fs::metadata(target).map(|m| m.len())
            && size > zeros.len() as u64
        {
            use std::io::Seek;
            let tail_offset = size - zeros.len() as u64;
            if f.seek(std::io::SeekFrom::Start(tail_offset)).is_ok() {
                let _ = f.write_all(&zeros);
            }
        }
        let _ = f.flush();
    }

    tracing::info!("disk signatures wiped on {}", target.display());
    Ok(())
}

/// Stream-decompress a `.raw.zst` file directly to a block device, calling `on_progress`
/// periodically with current write progress.
// r[impl installer.write.decompress-stream]
pub fn write_image(
    source: &Path,
    target: &Path,
    on_progress: &mut dyn FnMut(&WriteProgress),
) -> Result<()> {
    wipe_disk(target).context("wiping target disk before writing")?;

    let total_bytes = image_uncompressed_size(source).ok();

    let input =
        File::open(source).with_context(|| format!("opening source image {}", source.display()))?;

    let mut decoder = zstd::Decoder::new(input).context("initializing zstd decoder")?;

    let mut output = OpenOptions::new()
        .write(true)
        .open(target)
        .with_context(|| format!("opening target device {}", target.display()))?;

    let mut buf = vec![0u8; 4 * 1024 * 1024]; // 4 MiB buffer
    let mut bytes_written: u64 = 0;
    let start = Instant::now();

    loop {
        let n = decoder.read(&mut buf).context("reading from zstd stream")?;
        if n == 0 {
            break;
        }
        output
            .write_all(&buf[..n])
            .context("writing to target device")?;
        bytes_written += n as u64;

        on_progress(&WriteProgress {
            bytes_written,
            total_bytes,
            elapsed: start.elapsed(),
        });
    }

    output.flush().context("flushing target device")?;

    // Sync to ensure all data is physically written
    sync_device(&output)?;

    // Final progress callback with actual total
    on_progress(&WriteProgress {
        bytes_written,
        total_bytes: Some(bytes_written),
        elapsed: start.elapsed(),
    });

    tracing::info!(
        "wrote {} to {} in {:.1}s ({:.1} MiB/s)",
        format_size(bytes_written),
        target.display(),
        start.elapsed().as_secs_f64(),
        (bytes_written as f64) / (1024.0 * 1024.0) / start.elapsed().as_secs_f64(),
    );

    Ok(())
}

fn sync_device(file: &File) -> Result<()> {
    file.sync_all().context("syncing target device")?;
    Ok(())
}

fn format_size(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}

// r[impl installer.write.partitions]
/// Expand the GPT and root partition to fill the target disk.
///
/// After writing a fixed-size image to a larger disk, the GPT secondary header
/// is stranded in the middle and partition 3 (root) only covers the original
/// image size. This function:
///   1. Moves the GPT secondary header to the end of the disk.
///   2. Grows partition 3 to fill all remaining space.
///   3. Re-reads the partition table so the kernel picks up changes.
///
/// Filesystem-level resize (BTRFS, LUKS) is left to the boot-time
/// grow-root-filesystem service.
pub fn expand_partitions(target: &Path) -> Result<()> {
    let target_str = target.to_str().unwrap_or_default();

    tracing::info!("moving GPT secondary header on {}", target.display());
    let sgdisk_status = Command::new("sgdisk")
        .arg("--move-second-header")
        .arg(target)
        .output()
        .context("running sgdisk --move-second-header")?;
    if !sgdisk_status.status.success() {
        let stderr = String::from_utf8_lossy(&sgdisk_status.stderr);
        bail!("sgdisk --move-second-header failed: {stderr}");
    }

    reread_partition_table(target)?;

    tracing::info!(
        "growing root partition (partition 3) on {}",
        target.display()
    );
    let growpart_status = Command::new("growpart")
        .args(["--free-percent=1", target_str, "3"])
        .output()
        .context("running growpart")?;
    match growpart_status.status.code() {
        Some(0) => {
            tracing::info!("root partition grown successfully");
        }
        Some(1) => {
            tracing::info!("root partition already fills disk, no change needed");
        }
        _ => {
            let stderr = String::from_utf8_lossy(&growpart_status.stderr);
            bail!("growpart failed: {stderr}");
        }
    }

    reread_partition_table(target)?;

    tracing::info!("partition expansion complete on {}", target.display());
    Ok(())
}

// r[impl installer.write.partitions]
pub fn verify_partition_table(target: &Path) -> Result<()> {
    // Use sfdisk --json to read the GPT directly from the block device.
    // Unlike lsblk, this doesn't require the kernel to have created
    // partition device nodes, which makes it work inside containers.
    let output = Command::new("sfdisk")
        .args(["--json", target.to_str().unwrap_or_default()])
        .output()
        .context("running sfdisk --json to verify partitions")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("sfdisk verification failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    tracing::debug!("sfdisk output: {stdout}");

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).context("parsing sfdisk JSON output")?;

    let partitions = parsed
        .get("partitiontable")
        .and_then(|pt| pt.get("partitions"))
        .and_then(|p| p.as_array())
        .map(|a| a.as_slice())
        .unwrap_or_default();

    let found_labels: Vec<&str> = partitions
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
        .collect();

    let expected_labels = ["efi", "xboot", "root"];
    for label in &expected_labels {
        if !found_labels
            .iter()
            .any(|found| found.eq_ignore_ascii_case(label))
        {
            bail!(
                "partition verification failed: expected partition with label '{label}' not found (found: {found_labels:?})"
            );
        }
    }

    tracing::info!("partition table verified on {}", target.display());
    Ok(())
}

/// Re-read the partition table on the target device so the kernel picks up changes.
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

/// Ensure partition device nodes exist in `/dev` for the given disk.
///
/// Inside containers (e.g. systemd-nspawn), `partprobe` tells the kernel to
/// re-read the partition table and the kernel creates device nodes on the
/// *host's* devtmpfs, but the container has its own private `/dev` where
/// those nodes never appear. The container's `/sys` may also not expose
/// partition sub-entries.
///
/// This function reads sysfs (`/sys/class/block/<disk>/<partition>/dev`)
/// to discover partition sub-devices and their major:minor numbers, then
/// creates or recreates any `/dev` nodes that are missing or stale.
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

/// Try to create missing partition device nodes using sysfs entries.
/// Returns the number of nodes created.
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

/// Check whether `path` exists, is a block device, and has the expected major:minor.
///
/// Uses `MetadataExt::rdev()` to extract the device number and the libc
/// `major`/`minor` macros to split it, avoiding a subprocess.
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

/// Create a block device node. Returns 1 on success, 0 on failure (logged as warning).
///
/// If `dev_path` already exists but is not a valid block device with the right
/// major:minor, it is removed first so mknod can recreate it.
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

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.tui.progress]
    #[test]
    fn progress_fraction_with_total() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(10),
        };
        assert!((p.fraction().unwrap() - 0.5).abs() < f64::EPSILON);
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_fraction_without_total() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: None,
            elapsed: std::time::Duration::from_secs(10),
        };
        assert!(p.fraction().is_none());
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_eta_calculation() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(10),
        };
        let eta = p.eta().unwrap();
        assert!((eta.as_secs_f64() - 10.0).abs() < 0.1);
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_eta_at_zero() {
        let p = WriteProgress {
            bytes_written: 0,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(0),
        };
        assert!(p.eta().is_none());
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_eta_complete() {
        let p = WriteProgress {
            bytes_written: 1000,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(10),
        };
        let eta = p.eta().unwrap();
        assert!(eta.as_secs_f64() < 0.1);
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_throughput() {
        let p = WriteProgress {
            bytes_written: 10 * 1024 * 1024,
            total_bytes: None,
            elapsed: std::time::Duration::from_secs(1),
        };
        assert!((p.throughput_mbps() - 10.0).abs() < 0.1);
    }

    // r[verify installer.tui.progress]
    #[test]
    fn eta_formatting() {
        assert_eq!(format_eta(std::time::Duration::from_secs(45)), "45s");
        assert_eq!(format_eta(std::time::Duration::from_secs(90)), "1m30s");
        assert_eq!(format_eta(std::time::Duration::from_secs(3661)), "61m01s");
    }

    // r[verify installer.write.decompress-stream]
    #[test]
    fn size_formatting() {
        assert_eq!(format_size(0), "0.0 MiB");
        assert_eq!(format_size(1024 * 1024 * 512), "512.0 MiB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00 GiB");
        assert_eq!(format_size(8 * 1024 * 1024 * 1024), "8.00 GiB");
    }

    // r[verify installer.write.disk-size-check]
    #[test]
    fn check_disk_size_ok_when_equal() {
        assert!(check_disk_size(5 * 1024 * 1024 * 1024, 5 * 1024 * 1024 * 1024).is_ok());
    }

    // r[verify installer.write.disk-size-check]
    #[test]
    fn check_disk_size_ok_when_larger() {
        assert!(check_disk_size(5 * 1024 * 1024 * 1024, 10 * 1024 * 1024 * 1024).is_ok());
    }

    // r[verify installer.write.disk-size-check]
    #[test]
    fn check_disk_size_fails_when_too_small() {
        let result = check_disk_size(5 * 1024 * 1024 * 1024, 4 * 1024 * 1024 * 1024);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("too small"), "expected 'too small' in: {msg}");
        assert!(msg.contains("5.00 GiB"), "expected image size in: {msg}");
        assert!(msg.contains("4.00 GiB"), "expected disk size in: {msg}");
    }

    // r[verify installer.write.disk-size-check]
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

    // r[verify installer.write.disk-size-check]
    #[test]
    fn image_uncompressed_size_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let zst_path = dir.path().join("img.raw.zst");
        let size_path = dir.path().join("img.raw.size");

        std::fs::write(&zst_path, b"irrelevant").unwrap();
        std::fs::write(&size_path, "  1024  \n").unwrap();

        assert_eq!(image_uncompressed_size(&zst_path).unwrap(), 1024);
    }

    // r[verify installer.write.disk-size-check]
    #[test]
    fn image_uncompressed_size_fails_without_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let zst_path = dir.path().join("no-sidecar.raw.zst");
        std::fs::write(&zst_path, b"data").unwrap();

        assert!(image_uncompressed_size(&zst_path).is_err());
    }

    // r[verify installer.write.disk-size-check]
    #[test]
    fn image_uncompressed_size_fails_on_non_numeric() {
        let dir = tempfile::tempdir().unwrap();
        let zst_path = dir.path().join("bad.raw.zst");
        let size_path = dir.path().join("bad.raw.size");

        std::fs::write(&zst_path, b"data").unwrap();
        std::fs::write(&size_path, "not-a-number\n").unwrap();

        assert!(image_uncompressed_size(&zst_path).is_err());
    }

    // r[verify installer.write.disk-size-check]
    #[test]
    fn image_uncompressed_size_fails_without_zst_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.raw");

        assert!(image_uncompressed_size(&path).is_err());
    }
}
