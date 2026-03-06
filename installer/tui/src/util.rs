use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Derive a partition device path from a parent block device and a 1-based
/// partition number.
///
/// NVMe and loop devices use a `p` separator (`/dev/nvme0n1p3`,
/// `/dev/loop0p3`); SCSI/SATA disks append the number directly
/// (`/dev/sda3`). The rule: if the device path ends with an ASCII digit,
/// insert `p` before the partition number.
pub fn partition_path(device: &Path, part_num: u32) -> Result<PathBuf> {
    let dev_str = device.to_str().unwrap_or_default();

    let path = if dev_str.ends_with(|c: char| c.is_ascii_digit()) {
        PathBuf::from(format!("{dev_str}p{part_num}"))
    } else {
        PathBuf::from(format!("{dev_str}{part_num}"))
    };

    Ok(path)
}

/// Run an external program, log the invocation, capture output, and return a
/// contextual error on failure.
pub fn run_command(program: &str, args: &[&str]) -> Result<()> {
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

/// Write a passphrase to `/tmp/bes-luks-keyfile` with mode 0400 and return
/// the path. The caller is responsible for removing the file after use.
pub fn create_passphrase_keyfile(passphrase: &str) -> Result<PathBuf> {
    let path = PathBuf::from("/tmp/bes-luks-keyfile");
    fs::write(&path, passphrase.as_bytes()).context("creating passphrase keyfile")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400))
        .context("setting keyfile permissions")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.firstboot.mount+4]
    #[test]
    fn partition_path_scsi_disk() {
        let path = partition_path(Path::new("/dev/sda"), 3).unwrap();
        assert_eq!(path, PathBuf::from("/dev/sda3"));
    }

    // r[verify installer.firstboot.mount+4]
    #[test]
    fn partition_path_nvme() {
        let path = partition_path(Path::new("/dev/nvme0n1"), 3).unwrap();
        assert_eq!(path, PathBuf::from("/dev/nvme0n1p3"));
    }

    // r[verify installer.firstboot.mount+4]
    #[test]
    fn partition_path_loop() {
        let path = partition_path(Path::new("/dev/loop0"), 1).unwrap();
        assert_eq!(path, PathBuf::from("/dev/loop0p1"));
    }
}
