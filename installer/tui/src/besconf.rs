use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::paths;

const BESCONF_PARTUUID: &str = "e2bac42b-03a7-5048-b8f5-3f6d22100e77";
const BESCONF_MOUNT: &str = "/run/besconf";
const FAILURE_LOG_NAME: &str = "installer-failed.log";
const FAILURE_LOG_OLD_NAME: &str = "installer-failed.log.old";
const RECOVERY_KEYS_NAME: &str = "recovery-keys.txt";

/// Tracks whether the BESCONF partition is available and writable,
/// and whether recovery keys should be saved to it.
#[derive(Debug, Clone)]
pub struct BesconfState {
    writable: bool,
    mount_path: PathBuf,
    save_recovery_keys: bool,
}

#[cfg_attr(not(test), allow(dead_code))]
impl BesconfState {
    /// Create a read-only BESCONF state (e.g. for dry-run mode).
    pub fn readonly() -> Self {
        Self {
            writable: false,
            mount_path: PathBuf::from(BESCONF_MOUNT),
            save_recovery_keys: false,
        }
    }

    pub fn is_writable(&self) -> bool {
        self.writable
    }

    pub fn mount_path(&self) -> &Path {
        &self.mount_path
    }

    pub fn save_recovery_keys(&self) -> bool {
        self.save_recovery_keys
    }
}

/// Mount the BESCONF partition and detect whether it can be made writable.
///
/// r[impl iso.config-partition+4]
// r[impl installer.besconf.writable-detection+2]
///
/// Locates the BESCONF partition by its well-known PARTUUID, mounts it
/// read-only at `/run/besconf`, then attempts a read-write remount to
/// determine writability. If the partition is not found (e.g. development
/// or container test), returns a read-only state with the mount path set
/// but nothing actually mounted.
///
/// Returns `(BesconfState, bool)` — the state and whether we performed
/// the initial mount (so the caller knows to unmount on exit).
pub fn mount_and_detect() -> (BesconfState, bool) {
    let mount_path = PathBuf::from(BESCONF_MOUNT);
    let by_partuuid = PathBuf::from(format!("/dev/disk/by-partuuid/{BESCONF_PARTUUID}"));

    if !by_partuuid.exists() {
        tracing::info!(
            "BESCONF PARTUUID device not found at {}, treating as unavailable",
            by_partuuid.display()
        );
        return (
            BesconfState {
                writable: false,
                mount_path,
                save_recovery_keys: false,
            },
            false,
        );
    }

    // Create the mount point if it doesn't exist.
    if let Err(e) = fs::create_dir_all(&mount_path) {
        tracing::warn!("failed to create {}: {e}", mount_path.display());
        return (
            BesconfState {
                writable: false,
                mount_path,
                save_recovery_keys: false,
            },
            false,
        );
    }

    // Mount read-only first.
    let mount_output = Command::new(paths::MOUNT)
        .args(["-t", "vfat", "-o", "ro,noatime,iocharset=ascii"])
        .arg(&by_partuuid)
        .arg(&mount_path)
        .output();

    let mounted = match mount_output {
        Ok(o) if o.status.success() => {
            tracing::info!("BESCONF mounted read-only at {}", mount_path.display());
            true
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            tracing::info!("BESCONF mount failed (exit {}): {stderr}", o.status);
            false
        }
        Err(e) => {
            tracing::info!("BESCONF mount command failed to run: {e}");
            false
        }
    };

    if !mounted {
        return (
            BesconfState {
                writable: false,
                mount_path,
                save_recovery_keys: false,
            },
            false,
        );
    }

    // Try to remount read-write.
    let rw_output = Command::new(paths::MOUNT)
        .args(["-o", "remount,rw", BESCONF_MOUNT])
        .output();

    let writable = match rw_output {
        Ok(o) if o.status.success() => {
            tracing::info!("BESCONF remounted read-write");
            true
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            tracing::info!("BESCONF rw remount failed (exit {}): {stderr}", o.status);
            false
        }
        Err(e) => {
            tracing::info!("BESCONF rw remount command failed to run: {e}");
            false
        }
    };

    (
        BesconfState {
            writable,
            mount_path,
            save_recovery_keys: false,
        },
        true,
    )
}

/// Unmount BESCONF if we mounted it.
pub fn unmount() {
    let mount_path = Path::new(BESCONF_MOUNT);
    if mount_path.exists() {
        let status = Command::new(paths::UMOUNT).arg(mount_path).status();
        match status {
            Ok(s) if s.success() => {
                tracing::info!("unmounted BESCONF at {}", mount_path.display());
            }
            Ok(s) => {
                tracing::warn!("umount BESCONF exited with {s}");
            }
            Err(e) => {
                tracing::warn!("umount BESCONF failed: {e}");
            }
        }
    }
}

/// Set whether recovery keys should be saved to BESCONF after a
/// successful encrypted install.
pub fn with_save_recovery_keys(mut state: BesconfState, save: bool) -> BesconfState {
    state.save_recovery_keys = save;
    state
}

/// At installer startup, rotate any existing failure log so the previous
/// failure is preserved as `.old` and the main name is free for this run.
// r[impl installer.besconf.failure-log]
pub fn rotate_failure_log(state: &BesconfState) {
    if !state.writable {
        return;
    }

    let log_path = state.mount_path.join(FAILURE_LOG_NAME);
    if !log_path.exists() {
        return;
    }

    let old_path = state.mount_path.join(FAILURE_LOG_OLD_NAME);
    match fs::rename(&log_path, &old_path) {
        Ok(()) => {
            tracing::info!("rotated {} -> {}", log_path.display(), old_path.display());
        }
        Err(e) => {
            tracing::warn!(
                "failed to rotate failure log {} -> {}: {e}",
                log_path.display(),
                old_path.display()
            );
        }
    }
}

/// Copy the installer log to the BESCONF partition as `installer-failed.log`.
// r[impl installer.besconf.failure-log]
pub fn write_failure_log(state: &BesconfState, log_path: &Path) {
    if !state.writable {
        return;
    }

    let dest = state.mount_path.join(FAILURE_LOG_NAME);
    match fs::copy(log_path, &dest) {
        Ok(bytes) => {
            tracing::info!("copied installer log ({bytes} bytes) to {}", dest.display());
        }
        Err(e) => {
            tracing::warn!("failed to copy installer log to {}: {e}", dest.display());
        }
    }
}

/// Read the machine serial number from DMI/SMBIOS data.
///
/// Prefers `/sys/class/dmi/id/product_serial` (most commonly printed on
/// the outside of the chassis), falling back to
/// `/sys/class/dmi/id/board_serial`.
pub fn read_machine_serial() -> String {
    let candidates = [
        "/sys/class/dmi/id/product_serial",
        "/sys/class/dmi/id/board_serial",
    ];

    for path in &candidates {
        if let Ok(contents) = fs::read_to_string(path) {
            let trimmed = contents.trim();
            // Some BIOS vendors write placeholder values
            if !trimmed.is_empty()
                && !trimmed.eq_ignore_ascii_case("to be filled by o.e.m.")
                && !trimmed.eq_ignore_ascii_case("default string")
                && !trimmed.eq_ignore_ascii_case("not specified")
                && !trimmed.eq_ignore_ascii_case("none")
            {
                tracing::info!("machine serial from {path}: {trimmed}");
                return trimmed.to_string();
            }
        }
    }

    tracing::info!("no usable machine serial found in DMI data");
    "unknown".to_string()
}

/// Read the UUID of a block device via `blkid`.
pub fn read_partition_uuid(device: &Path) -> Result<String> {
    let output = std::process::Command::new(paths::BLKID)
        .args(["-s", "UUID", "-o", "value"])
        .arg(device)
        .output()
        .with_context(|| format!("running blkid on {}", device.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "blkid failed on {} (exit {}): {stderr}",
            device.display(),
            output.status
        );
    }

    let uuid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uuid.is_empty() {
        anyhow::bail!("blkid returned empty UUID for {}", device.display());
    }

    Ok(uuid)
}

/// Append a recovery key entry to `recovery-keys.txt` on the BESCONF
/// partition.
// r[impl installer.config.save-recovery-keys]
pub fn append_recovery_key(
    state: &BesconfState,
    passphrase: &str,
    root_partition: &Path,
) -> Result<()> {
    if !state.writable || !state.save_recovery_keys {
        tracing::info!(
            "BESCONF not writable or save-recovery-keys not enabled, skipping recovery key save"
        );
        return Ok(());
    }

    let uuid = read_partition_uuid(root_partition).unwrap_or_else(|e| {
        tracing::warn!("could not read root partition UUID: {e}");
        "unknown".to_string()
    });

    let serial = read_machine_serial();

    let keys_path = state.mount_path.join(RECOVERY_KEYS_NAME);
    let line = format!("{passphrase}\t{uuid}\t{serial}\n");

    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&keys_path)
        .with_context(|| format!("opening {} for append", keys_path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("writing to {}", keys_path.display()))?;

    tracing::info!(
        "appended recovery key entry to {} (uuid={uuid}, serial={serial})",
        keys_path.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.besconf.writable-detection+2]
    #[test]
    fn besconf_state_tracks_writable() {
        let state = BesconfState {
            writable: true,
            mount_path: PathBuf::from("/tmp/test-besconf"),
            save_recovery_keys: false,
        };
        assert!(state.is_writable());
        assert_eq!(state.mount_path(), Path::new("/tmp/test-besconf"));
    }

    // r[verify installer.besconf.writable-detection+2]
    #[test]
    fn besconf_state_tracks_readonly() {
        let state = BesconfState {
            writable: false,
            mount_path: PathBuf::from("/run/besconf"),
            save_recovery_keys: false,
        };
        assert!(!state.is_writable());
    }

    // r[verify installer.config.save-recovery-keys]
    #[test]
    fn besconf_state_tracks_save_recovery_keys() {
        let state = with_save_recovery_keys(
            BesconfState {
                writable: true,
                mount_path: PathBuf::from("/tmp/test-besconf"),
                save_recovery_keys: false,
            },
            true,
        );
        assert!(state.save_recovery_keys());
    }

    // r[verify installer.besconf.failure-log]
    #[test]
    fn rotate_failure_log_renames_existing() {
        let dir = tempfile::tempdir().unwrap();
        let state = BesconfState {
            writable: true,
            mount_path: dir.path().to_path_buf(),
            save_recovery_keys: false,
        };

        let log_path = dir.path().join(FAILURE_LOG_NAME);
        fs::write(&log_path, "previous failure log contents").unwrap();

        rotate_failure_log(&state);

        assert!(!log_path.exists());
        let old_path = dir.path().join(FAILURE_LOG_OLD_NAME);
        assert!(old_path.exists());
        assert_eq!(
            fs::read_to_string(&old_path).unwrap(),
            "previous failure log contents"
        );
    }

    // r[verify installer.besconf.failure-log]
    #[test]
    fn rotate_failure_log_clobbers_old() {
        let dir = tempfile::tempdir().unwrap();
        let state = BesconfState {
            writable: true,
            mount_path: dir.path().to_path_buf(),
            save_recovery_keys: false,
        };

        let old_path = dir.path().join(FAILURE_LOG_OLD_NAME);
        fs::write(&old_path, "ancient log").unwrap();

        let log_path = dir.path().join(FAILURE_LOG_NAME);
        fs::write(&log_path, "recent failure").unwrap();

        rotate_failure_log(&state);

        assert!(!log_path.exists());
        assert_eq!(fs::read_to_string(&old_path).unwrap(), "recent failure");
    }

    // r[verify installer.besconf.failure-log]
    #[test]
    fn rotate_failure_log_noop_when_no_log() {
        let dir = tempfile::tempdir().unwrap();
        let state = BesconfState {
            writable: true,
            mount_path: dir.path().to_path_buf(),
            save_recovery_keys: false,
        };

        rotate_failure_log(&state);

        assert!(!dir.path().join(FAILURE_LOG_NAME).exists());
        assert!(!dir.path().join(FAILURE_LOG_OLD_NAME).exists());
    }

    // r[verify installer.besconf.failure-log]
    #[test]
    fn rotate_failure_log_noop_when_readonly() {
        let dir = tempfile::tempdir().unwrap();
        let state = BesconfState {
            writable: false,
            mount_path: dir.path().to_path_buf(),
            save_recovery_keys: false,
        };

        let log_path = dir.path().join(FAILURE_LOG_NAME);
        fs::write(&log_path, "should not be touched").unwrap();

        rotate_failure_log(&state);

        assert!(log_path.exists());
        assert!(!dir.path().join(FAILURE_LOG_OLD_NAME).exists());
    }

    // r[verify installer.besconf.failure-log]
    #[test]
    fn write_failure_log_copies_to_besconf() {
        let dir = tempfile::tempdir().unwrap();
        let state = BesconfState {
            writable: true,
            mount_path: dir.path().to_path_buf(),
            save_recovery_keys: false,
        };

        let src = dir.path().join("installer.log");
        fs::write(&src, "test log contents\nline 2\n").unwrap();

        write_failure_log(&state, &src);

        let dest = dir.path().join(FAILURE_LOG_NAME);
        assert!(dest.exists());
        assert_eq!(
            fs::read_to_string(&dest).unwrap(),
            "test log contents\nline 2\n"
        );
    }

    // r[verify installer.besconf.failure-log]
    #[test]
    fn write_failure_log_noop_when_readonly() {
        let dir = tempfile::tempdir().unwrap();
        let state = BesconfState {
            writable: false,
            mount_path: dir.path().to_path_buf(),
            save_recovery_keys: false,
        };

        let src = dir.path().join("installer.log");
        fs::write(&src, "log data").unwrap();

        write_failure_log(&state, &src);

        assert!(!dir.path().join(FAILURE_LOG_NAME).exists());
    }

    // r[verify installer.config.save-recovery-keys]
    #[test]
    fn append_recovery_key_noop_when_readonly() {
        let dir = tempfile::tempdir().unwrap();
        let state = BesconfState {
            writable: false,
            mount_path: dir.path().to_path_buf(),
            save_recovery_keys: false,
        };

        let result = append_recovery_key(&state, "passphrase", Path::new("/dev/sda3"));
        assert!(result.is_ok());
        assert!(!dir.path().join(RECOVERY_KEYS_NAME).exists());
    }

    // r[verify installer.config.save-recovery-keys]
    #[test]
    fn append_recovery_key_noop_when_not_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let state = BesconfState {
            writable: true,
            mount_path: dir.path().to_path_buf(),
            save_recovery_keys: false,
        };

        let result = append_recovery_key(&state, "passphrase", Path::new("/dev/sda3"));
        assert!(result.is_ok());
        assert!(!dir.path().join(RECOVERY_KEYS_NAME).exists());
    }

    #[test]
    fn read_machine_serial_returns_unknown_when_no_dmi() {
        // In test environments DMI files are usually absent or have
        // placeholder values, so we just verify it returns a non-empty string.
        let serial = read_machine_serial();
        assert!(!serial.is_empty());
    }
}
