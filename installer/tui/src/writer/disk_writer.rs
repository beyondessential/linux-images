use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};

use crate::config::DiskEncryption;
use crate::paths;

use super::device::{partition_path, reread_partition_table, run_command, sync_device};
use super::luks::{
    LUKS_NAME, close_luks_root, create_passphrase_keyfile, format_luks_for_root, open_luks_root,
};
use super::manifest::{PartitionManifest, image_size, partition_images_total_size};
use super::progress::{WriteProgress, format_size};
use super::verity::splice_fd_to_fd;

const MOUNT_BASE: &str = "/mnt/target";

pub struct DiskWriter<'a> {
    pub target: &'a Path,
    pub disk_encryption: DiskEncryption,
    pub passphrase: Option<&'a str>,
}

impl<'a> DiskWriter<'a> {
    pub fn new(
        target: &'a Path,
        disk_encryption: DiskEncryption,
        passphrase: Option<&'a str>,
    ) -> Self {
        Self {
            target,
            disk_encryption,
            passphrase,
        }
    }

    // r[impl installer.write.partitions+2]
    pub fn wipe_disk(&self) -> Result<()> {
        tracing::info!("wiping existing signatures on {}", self.target.display());

        let wipefs_status = Command::new(paths::WIPEFS)
            .args(["--all", "--force"])
            .arg(self.target)
            .output()
            .context("running wipefs")?;

        if !wipefs_status.status.success() {
            let stderr = String::from_utf8_lossy(&wipefs_status.stderr);
            tracing::warn!("wipefs failed (non-fatal): {stderr}");
        }

        let sgdisk_status = Command::new(paths::SGDISK)
            .arg("--zap-all")
            .arg(self.target)
            .output()
            .context("running sgdisk --zap-all")?;

        if !sgdisk_status.status.success() {
            let stderr = String::from_utf8_lossy(&sgdisk_status.stderr);
            tracing::warn!("sgdisk --zap-all failed (non-fatal): {stderr}");
        }

        if let Ok(mut f) = OpenOptions::new().write(true).open(self.target) {
            let zeros = vec![0u8; 1024 * 1024];
            let _ = f.write_all(&zeros);

            if let Ok(size) = fs::metadata(self.target).map(|m| m.len())
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

        tracing::info!("disk signatures wiped on {}", self.target.display());
        Ok(())
    }

    // r[impl installer.write.partitions+2]
    pub fn create_partition_table(&self, manifest: &PartitionManifest) -> Result<()> {
        tracing::info!(
            "creating GPT with {} partitions on {}",
            manifest.partitions.len(),
            self.target.display()
        );

        let target_str = self.target.to_str().unwrap_or_default();

        let mut args: Vec<String> = vec!["--clear".to_string()];

        for (i, entry) in manifest.partitions.iter().enumerate() {
            let part_num = i + 1;
            let size_spec = if entry.size_mib == 0 {
                format!("-n{part_num}:0:0")
            } else {
                format!("-n{part_num}:0:+{}M", entry.size_mib)
            };
            args.push(size_spec);
            args.push(format!("-t{part_num}:{}", entry.type_uuid));
            args.push(format!("-c{part_num}:{}", entry.label));
        }

        args.push(target_str.to_string());

        let output = Command::new(paths::SGDISK)
            .args(&args)
            .output()
            .context("running sgdisk to create partition table")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("sgdisk failed: {stderr}");
        }

        tracing::info!("partition table created on {}", self.target.display());
        Ok(())
    }

    // r[impl installer.write.stream-copy]
    fn splice_to_device(
        &self,
        source: &Path,
        device: &Path,
        bytes_offset: u64,
        total_bytes: Option<u64>,
        start: Instant,
        on_progress: &mut dyn FnMut(&WriteProgress),
    ) -> Result<u64> {
        let input = File::open(source)
            .with_context(|| format!("opening source image {}", source.display()))?;
        let expected = image_size(source).ok();

        let output = OpenOptions::new()
            .write(true)
            .open(device)
            .with_context(|| format!("opening target device {}", device.display()))?;

        let partition_bytes_written = splice_fd_to_fd(
            input.as_raw_fd(),
            output.as_raw_fd(),
            expected,
            bytes_offset,
            total_bytes,
            start,
            on_progress,
        )
        .with_context(|| format!("splicing {} -> {}", source.display(), device.display()))?;

        sync_device(&output)?;

        tracing::info!(
            "wrote {} to {} in {:.1}s ({:.1} MiB/s)",
            format_size(partition_bytes_written),
            device.display(),
            start.elapsed().as_secs_f64(),
            if start.elapsed().as_secs_f64() > 0.0 {
                (partition_bytes_written as f64) / (1024.0 * 1024.0) / start.elapsed().as_secs_f64()
            } else {
                0.0
            },
        );

        Ok(partition_bytes_written)
    }

    // r[impl installer.write.partitions+2]
    // r[impl installer.write.stream-copy]
    pub fn write_partitions(
        &self,
        manifest: &PartitionManifest,
        images_dir: &Path,
        on_progress: &mut dyn FnMut(&WriteProgress),
    ) -> Result<()> {
        let total_bytes = partition_images_total_size(manifest, images_dir).ok();

        self.wipe_disk()
            .context("wiping target disk before writing")?;
        self.create_partition_table(manifest)
            .context("creating partition table")?;
        reread_partition_table(self.target).context("re-reading partition table after creation")?;

        let start = Instant::now();
        let mut bytes_offset: u64 = 0;

        for (i, entry) in manifest.partitions.iter().enumerate() {
            let part_num = (i + 1) as u32;
            let part_device = partition_path(self.target, part_num)?;
            let img_path = images_dir.join(&entry.image);

            tracing::info!(
                "writing {} -> {} (partition {})",
                entry.image,
                part_device.display(),
                part_num,
            );

            let write_device = if entry.label == "root" && self.disk_encryption.is_encrypted() {
                format_luks_for_root(&part_device, self.passphrase.unwrap_or_default())
                    .context("formatting LUKS on root partition")?
            } else {
                part_device.clone()
            };

            let written = self
                .splice_to_device(
                    &img_path,
                    &write_device,
                    bytes_offset,
                    total_bytes,
                    start,
                    on_progress,
                )
                .with_context(|| format!("writing partition {}", entry.label))?;

            bytes_offset += written;

            if entry.label == "root" && self.disk_encryption.is_encrypted() {
                close_luks_root().context("closing LUKS after writing root")?;
            }
        }

        on_progress(&WriteProgress {
            bytes_written: bytes_offset,
            total_bytes: Some(bytes_offset),
            elapsed: start.elapsed(),
        });

        tracing::info!("all partitions written to {}", self.target.display());
        Ok(())
    }

    // r[impl installer.write.expand-root]
    pub fn expand_root_filesystem(&self) -> Result<()> {
        let root_part = partition_path(self.target, 3)?;

        let btrfs_dev = if self.disk_encryption.is_encrypted() {
            let pp = self.passphrase.unwrap_or_default();
            let mapper_dev = open_luks_root(&root_part, pp)?;

            tracing::info!("resizing LUKS container to fill partition");
            let keyfile = create_passphrase_keyfile(pp)?;
            let output = Command::new(paths::CRYPTSETUP)
                .args([
                    "resize",
                    "--key-file",
                    keyfile.to_str().unwrap_or_default(),
                    LUKS_NAME,
                ])
                .output()
                .context("running cryptsetup resize")?;
            let _ = fs::remove_file(&keyfile);
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("cryptsetup resize failed: {stderr}");
            }

            mapper_dev
        } else {
            root_part.clone()
        };

        let mount_path = PathBuf::from(MOUNT_BASE);
        fs::create_dir_all(&mount_path).context("creating mount point")?;

        run_command(
            paths::MOUNT,
            &[
                "-t",
                "btrfs",
                "-o",
                "subvol=@",
                btrfs_dev.to_str().unwrap_or_default(),
                mount_path.to_str().unwrap_or_default(),
            ],
        )
        .context("mounting btrfs for resize")?;

        tracing::info!("resizing btrfs filesystem to fill partition");
        let resize_result = run_command(
            paths::BTRFS,
            &[
                "filesystem",
                "resize",
                "max",
                mount_path.to_str().unwrap_or_default(),
            ],
        );

        let _ = run_command(paths::UMOUNT, &[mount_path.to_str().unwrap_or_default()]);

        if self.disk_encryption.is_encrypted() {
            let _ = close_luks_root();
        }

        resize_result.context("resizing btrfs filesystem")?;

        tracing::info!("root filesystem expanded");
        Ok(())
    }

    // r[impl installer.write.randomize-uuids+2]
    pub fn randomize_filesystem_uuids(&self) -> Result<()> {
        tracing::info!("randomizing filesystem UUIDs on {}", self.target.display());

        let efi_part = partition_path(self.target, 1)?;
        let xboot_part = partition_path(self.target, 2)?;
        let root_part = partition_path(self.target, 3)?;

        // Ensure partition device nodes exist inside the container — they may
        // have been removed or never created if a previous step didn't trigger
        // sysfs-based mknod (e.g. in nspawn with private /dev).
        super::device::ensure_partition_devices(self.target)
            .context("ensuring partition devices before UUID randomization")?;

        match Command::new(paths::MLABEL)
            .args(["-n", "-i", efi_part.to_str().unwrap_or_default(), "::"])
            .output()
        {
            Ok(efi_result) if !efi_result.status.success() => {
                let stderr = String::from_utf8_lossy(&efi_result.stderr);
                let stdout = String::from_utf8_lossy(&efi_result.stdout);
                tracing::warn!("mlabel failed (non-fatal): stderr={stderr} stdout={stdout}");
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!("mlabel not found (non-fatal): EFI serial not randomized");
            }
            Err(e) => {
                tracing::warn!("mlabel failed (non-fatal): {e}");
            }
        }

        // tune2fs -U requires a freshly checked filesystem; run e2fsck first.
        let e2fsck_result = Command::new(paths::E2FSCK)
            .args(["-f", "-y", xboot_part.to_str().unwrap_or_default()])
            .output()
            .context("running e2fsck on xboot before UUID randomization")?;
        if !e2fsck_result.status.success() {
            let stderr = String::from_utf8_lossy(&e2fsck_result.stderr);
            let stdout = String::from_utf8_lossy(&e2fsck_result.stdout);
            tracing::warn!(
                "e2fsck on xboot exited with {} (non-fatal): stderr={stderr} stdout={stdout}",
                e2fsck_result.status,
            );
        }

        let xboot_result = Command::new(paths::TUNE2FS)
            .args(["-U", "random", xboot_part.to_str().unwrap_or_default()])
            .output()
            .context("running tune2fs to randomize xboot UUID")?;
        if !xboot_result.status.success() {
            let stderr = String::from_utf8_lossy(&xboot_result.stderr);
            let stdout = String::from_utf8_lossy(&xboot_result.stdout);
            bail!(
                "tune2fs -U random failed on xboot (exit {}): stderr={stderr} stdout={stdout}",
                xboot_result.status
            );
        }

        let btrfs_dev = if self.disk_encryption.is_encrypted() {
            open_luks_root(&root_part, self.passphrase.unwrap_or_default())?
        } else {
            root_part.clone()
        };

        let btrfs_result = Command::new(paths::BTRFSTUNE)
            .args(["-f", "-u", btrfs_dev.to_str().unwrap_or_default()])
            .output()
            .context("running btrfstune to randomize root UUID")?;

        if self.disk_encryption.is_encrypted() {
            let _ = close_luks_root();
        }

        if !btrfs_result.status.success() {
            let stderr = String::from_utf8_lossy(&btrfs_result.stderr);
            let stdout = String::from_utf8_lossy(&btrfs_result.stdout);
            bail!(
                "btrfstune -u failed on root (exit {}): stderr={stderr} stdout={stdout}",
                btrfs_result.status
            );
        }

        tracing::info!("filesystem UUIDs randomized");
        Ok(())
    }

    // r[impl installer.write.rebuild-boot-config+3]
    pub fn rebuild_boot_config(&self) -> Result<()> {
        tracing::info!("rebuilding boot config (initramfs + grub)");

        let root_part = partition_path(self.target, 3)?;
        let xboot_part = partition_path(self.target, 2)?;
        let efi_part = partition_path(self.target, 1)?;
        let is_encrypted = self.disk_encryption.is_encrypted();

        let luks_uuid = if is_encrypted {
            Some(
                blkid_value(&root_part, "UUID")
                    .context("reading LUKS UUID from raw root partition")?,
            )
        } else {
            None
        };

        let luks_opened = if is_encrypted {
            let _ = open_luks_root(&root_part, self.passphrase.unwrap_or_default())?;
            true
        } else {
            false
        };

        let btrfs_dev = if luks_opened {
            PathBuf::from(format!("/dev/mapper/{LUKS_NAME}"))
        } else {
            root_part.clone()
        };

        let root_uuid = blkid_value(&btrfs_dev, "UUID").context("reading root filesystem UUID")?;
        let xboot_uuid =
            blkid_value(&xboot_part, "UUID").context("reading xboot filesystem UUID")?;

        let mount_path = PathBuf::from(MOUNT_BASE);
        fs::create_dir_all(&mount_path).context("creating mount point")?;

        run_command(
            paths::MOUNT,
            &[
                "-t",
                "btrfs",
                "-o",
                "subvol=@,compress=zstd:6",
                btrfs_dev.to_str().unwrap_or_default(),
                mount_path.to_str().unwrap_or_default(),
            ],
        )
        .context("mounting btrfs root for boot config rebuild")?;

        let xboot_mount = mount_path.join("boot");
        fs::create_dir_all(&xboot_mount).ok();
        let mount_xboot_result = run_command(
            paths::MOUNT,
            &[
                xboot_part.to_str().unwrap_or_default(),
                xboot_mount.to_str().unwrap_or_default(),
            ],
        );
        let xboot_mounted = mount_xboot_result.is_ok();

        let efi_mount = mount_path.join("boot/efi");
        fs::create_dir_all(&efi_mount).ok();
        let mount_efi_result = run_command(
            paths::MOUNT,
            &[
                efi_part.to_str().unwrap_or_default(),
                efi_mount.to_str().unwrap_or_default(),
            ],
        );
        let efi_mounted = mount_efi_result.is_ok();

        let proc_path = mount_path.join("proc");
        let sys_path = mount_path.join("sys");
        let dev_path = mount_path.join("dev");

        let _ = run_command(
            paths::MOUNT,
            &["--bind", "/proc", proc_path.to_str().unwrap_or_default()],
        );
        let _ = run_command(
            paths::MOUNT,
            &["--bind", "/sys", sys_path.to_str().unwrap_or_default()],
        );
        let _ = run_command(
            paths::MOUNT,
            &["--bind", "/dev", dev_path.to_str().unwrap_or_default()],
        );

        let mount_str = mount_path.to_str().unwrap_or_default();

        let modules_dir = mount_path.join("lib/modules");
        let kernel_version = if modules_dir.exists() {
            let mut versions: Vec<String> = fs::read_dir(&modules_dir)
                .context("reading /lib/modules in target")?
                .filter_map(|entry| {
                    entry.ok().and_then(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        if e.path().is_dir() { Some(name) } else { None }
                    })
                })
                .collect();
            versions.sort();
            versions.pop()
        } else {
            None
        };

        let dracut_result = if let Some(ref kver) = kernel_version {
            tracing::info!("rebuilding initramfs for kernel {kver}");
            run_command(
                paths::CHROOT,
                &[mount_str, paths::DRACUT, "--force", "--kver", kver],
            )
        } else {
            tracing::warn!("no kernel version found in target, running dracut without --kver");
            run_command(
                paths::CHROOT,
                &[mount_str, paths::DRACUT, "--force", "--regenerate-all"],
            )
        };

        if let Some(ref luks_id) = luks_uuid {
            patch_grub_defaults_for_luks(&mount_path, luks_id)
                .context("patching /etc/default/grub for LUKS")?;
        }

        let grub_probe_backup = install_grub_probe_wrapper(
            &mount_path,
            &btrfs_dev,
            &root_uuid,
            &xboot_part,
            &xboot_uuid,
            &efi_part,
            is_encrypted,
        )
        .context("installing grub-probe wrapper")?;

        let grub_result = run_command(paths::CHROOT, &[mount_str, "update-grub"]);

        remove_grub_probe_wrapper(&mount_path, grub_probe_backup);

        let _ = run_command(paths::UMOUNT, &[dev_path.to_str().unwrap_or_default()]);
        let _ = run_command(paths::UMOUNT, &[sys_path.to_str().unwrap_or_default()]);
        let _ = run_command(paths::UMOUNT, &[proc_path.to_str().unwrap_or_default()]);
        if efi_mounted {
            let _ = run_command(paths::UMOUNT, &[efi_mount.to_str().unwrap_or_default()]);
        }
        if xboot_mounted {
            let _ = run_command(paths::UMOUNT, &[xboot_mount.to_str().unwrap_or_default()]);
        }
        let _ = run_command(paths::UMOUNT, &[mount_path.to_str().unwrap_or_default()]);

        if luks_opened {
            let _ = close_luks_root();
        }

        dracut_result.context("rebuilding initramfs with dracut in chroot")?;
        grub_result.context("running update-grub in chroot")?;

        tracing::info!("boot config rebuilt successfully");
        Ok(())
    }

    // r[impl installer.write.partitions+2]
    pub fn verify_partition_table(&self) -> Result<()> {
        let output = Command::new(paths::SFDISK)
            .args(["--json", self.target.to_str().unwrap_or_default()])
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

        tracing::info!("partition table verified on {}", self.target.display());
        Ok(())
    }
}

fn blkid_value(device: &Path, tag: &str) -> Result<String> {
    let output = Command::new(paths::BLKID)
        .args([
            "-s",
            tag,
            "-o",
            "value",
            device.to_str().unwrap_or_default(),
        ])
        .output()
        .with_context(|| format!("running blkid on {}", device.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("blkid failed on {}: {stderr}", device.display());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Patch `/etc/default/grub` inside the target so `update-grub` produces a
/// kernel command line that tells dracut to unlock the LUKS root volume.
fn patch_grub_defaults_for_luks(mount_path: &Path, luks_uuid: &str) -> Result<()> {
    let grub_defaults = mount_path.join("etc/default/grub");
    let contents = fs::read_to_string(&grub_defaults).context("reading /etc/default/grub")?;

    let luks_cmdline = format!("rd.luks.name={luks_uuid}=root rd.luks.options=discard");

    let mut new_lines: Vec<String> = Vec::new();
    let mut found_cmdline_linux = false;
    let mut found_cmdline_default = false;

    for line in contents.lines() {
        if line.starts_with("GRUB_CMDLINE_LINUX=")
            && !line.starts_with("GRUB_CMDLINE_LINUX_DEFAULT=")
        {
            new_lines.push(format!("GRUB_CMDLINE_LINUX=\"{luks_cmdline}\""));
            found_cmdline_linux = true;
        } else if line.starts_with("GRUB_CMDLINE_LINUX_DEFAULT=") {
            // Remove serial console for encrypted installs
            let patched = line
                .replace("console=ttyS0,115200n8", "")
                .replace("  ", " ");
            // Clean up any trailing/leading spaces inside the quotes
            let patched = patched.replace("\" ", "\"").replace(" \"", "\"");
            new_lines.push(patched);
            found_cmdline_default = true;
        } else {
            new_lines.push(line.to_string());
        }
    }

    if !found_cmdline_linux {
        new_lines.push(format!("GRUB_CMDLINE_LINUX=\"{luks_cmdline}\""));
    }

    // Ensure there's a trailing newline
    let mut output = new_lines.join("\n");
    if !output.ends_with('\n') {
        output.push('\n');
    }

    fs::write(&grub_defaults, &output).context("writing /etc/default/grub")?;

    if found_cmdline_default {
        tracing::info!("patched GRUB_CMDLINE_LINUX_DEFAULT (removed serial console)");
    }
    tracing::info!("set GRUB_CMDLINE_LINUX for LUKS: {luks_cmdline}");

    Ok(())
}

/// Install a temporary wrapper script that replaces `grub-probe` inside the
/// chroot so that `update-grub` (which calls `grub-mkconfig`, which calls
/// `grub-probe`) can resolve the root and boot devices.
///
/// In a chroot environment, `/proc/self/mountinfo` still reflects the host
/// mount namespace, so `grub-probe --target=device /` fails with "cannot
/// find a device for /". The wrapper intercepts the queries that
/// `grub-mkconfig` makes and returns the correct values.
///
/// Returns the path to the backup of the real grub-probe (if any) so the
/// caller can restore it.
fn install_grub_probe_wrapper(
    mount_path: &Path,
    root_dev: &Path,
    root_uuid: &str,
    xboot_dev: &Path,
    xboot_uuid: &str,
    efi_dev: &Path,
    is_encrypted: bool,
) -> Result<Option<PathBuf>> {
    use std::os::unix::fs::PermissionsExt;

    let probe_path = mount_path.join("usr/sbin/grub-probe");
    let backup_path = mount_path.join("usr/sbin/grub-probe.real");

    let backup = if probe_path.exists() {
        fs::rename(&probe_path, &backup_path).context("backing up grub-probe")?;
        Some(backup_path.clone())
    } else {
        None
    };

    let root_dev_str = root_dev.to_str().unwrap_or_default();
    let xboot_dev_str = xboot_dev.to_str().unwrap_or_default();
    let efi_dev_str = efi_dev.to_str().unwrap_or_default();

    let wrapper = format!(
        r##"#!/bin/sh
# Temporary grub-probe wrapper installed by bes-installer.
# Handles the queries that grub-mkconfig needs to generate grub.cfg.
# Falls back to the real grub-probe for anything else.

REAL_PROBE="/usr/sbin/grub-probe.real"

# Parse arguments: we need to handle:
#   grub-probe --target=device /
#   grub-probe --target=device /boot
#   grub-probe --device DEVICE --target=fs_uuid
#   grub-probe --device DEVICE --target=fs
#   grub-probe --device DEVICE --target=partuuid
#   grub-probe --target=fs /
#   grub-probe --target=abstraction /

TARGET=""
DEVICE=""
PROBE_PATH=""

while [ $# -gt 0 ]; do
    case "$1" in
        --target=*) TARGET="${{1#--target=}}" ;;
        --target) shift; TARGET="$1" ;;
        --device) shift; DEVICE="$1" ;;
        --device=*) DEVICE="${{1#--device=}}" ;;
        -*) ;;
        *) PROBE_PATH="$1" ;;
    esac
    shift
done

# Path-based queries (grub-probe --target=X /path)
if [ -n "$PROBE_PATH" ]; then
    case "$PROBE_PATH" in
        /|/.)
            case "$TARGET" in
                device) echo "{root_dev_str}"; exit 0 ;;
                fs_uuid) echo "{root_uuid}"; exit 0 ;;
                fs) echo "btrfs"; exit 0 ;;
                abstraction) echo "{root_abstraction}"; exit 0 ;;
                partuuid) ;; # fall through to real probe
                *) ;;
            esac
            ;;
        /boot|/boot/)
            case "$TARGET" in
                device) echo "{xboot_dev_str}"; exit 0 ;;
                fs_uuid) echo "{xboot_uuid}"; exit 0 ;;
                fs) echo "ext2"; exit 0 ;;
                abstraction) echo ""; exit 0 ;;
                *) ;;
            esac
            ;;
        /boot/efi|/boot/efi/)
            case "$TARGET" in
                device) echo "{efi_dev_str}"; exit 0 ;;
                fs) echo "fat"; exit 0 ;;
                *) ;;
            esac
            ;;
    esac
fi

# Device-based queries (grub-probe --device DEV --target=X)
if [ -n "$DEVICE" ]; then
    case "$DEVICE" in
        {root_dev_str})
            case "$TARGET" in
                fs_uuid) echo "{root_uuid}"; exit 0 ;;
                fs) echo "btrfs"; exit 0 ;;
                abstraction) echo "{root_abstraction}"; exit 0 ;;
                *) ;;
            esac
            ;;
        {xboot_dev_str})
            case "$TARGET" in
                fs_uuid) echo "{xboot_uuid}"; exit 0 ;;
                fs) echo "ext2"; exit 0 ;;
                *) ;;
            esac
            ;;
        {efi_dev_str})
            case "$TARGET" in
                fs) echo "fat"; exit 0 ;;
                *) ;;
            esac
            ;;
    esac
fi

# Fall back to real grub-probe if available
if [ -x "$REAL_PROBE" ]; then
    exec "$REAL_PROBE" "$@"
fi

echo "grub-probe wrapper: unhandled query target=$TARGET device=$DEVICE path=$PROBE_PATH" >&2
exit 1
"##,
        root_dev_str = root_dev_str,
        root_uuid = root_uuid,
        root_abstraction = if is_encrypted { "luks" } else { "" },
        xboot_dev_str = xboot_dev_str,
        xboot_uuid = xboot_uuid,
        efi_dev_str = efi_dev_str,
    );

    fs::write(&probe_path, wrapper).context("writing grub-probe wrapper")?;
    fs::set_permissions(&probe_path, fs::Permissions::from_mode(0o755))
        .context("setting grub-probe wrapper permissions")?;

    tracing::info!("installed grub-probe wrapper in chroot");
    Ok(backup)
}

fn remove_grub_probe_wrapper(mount_path: &Path, backup: Option<PathBuf>) {
    let probe_path = mount_path.join("usr/sbin/grub-probe");
    if let Some(backup_path) = backup {
        if let Err(e) = fs::rename(&backup_path, &probe_path) {
            tracing::warn!("failed to restore grub-probe from backup: {e}");
        } else {
            tracing::info!("restored original grub-probe");
        }
    } else {
        let _ = fs::remove_file(&probe_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.write.partitions+2]
    #[test]
    fn create_partition_table_builds_correct_sgdisk_args() {
        let manifest = PartitionManifest {
            arch: "amd64".into(),
            partitions: vec![
                super::super::manifest::PartitionEntry {
                    label: "efi".into(),
                    type_uuid: "C12A7328-F81F-11D2-BA4B-00A0C93EC93B".into(),
                    size_mib: 512,
                    image: "efi.img.zst".into(),
                },
                super::super::manifest::PartitionEntry {
                    label: "xboot".into(),
                    type_uuid: "BC13C2FF-59E6-4262-A352-B275FD6F7172".into(),
                    size_mib: 1024,
                    image: "xboot.img.zst".into(),
                },
                super::super::manifest::PartitionEntry {
                    label: "root".into(),
                    type_uuid: "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709".into(),
                    size_mib: 0,
                    image: "root.img.zst".into(),
                },
            ],
        };

        assert_eq!(manifest.partitions.len(), 3);
        assert_eq!(manifest.partitions[0].size_mib, 512);
        assert_eq!(manifest.partitions[2].size_mib, 0);
    }

    // r[verify installer.write.rebuild-boot-config+3]
    #[test]
    fn patch_grub_defaults_sets_luks_cmdline() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc/default");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("grub"),
            "GRUB_DEFAULT=0\n\
             GRUB_CMDLINE_LINUX_DEFAULT=\"noresume console=ttyS0,115200n8\"\n\
             GRUB_CMDLINE_LINUX=\"\"\n",
        )
        .unwrap();

        let luks_uuid = "abcd1234-5678-9abc-def0-123456789abc";
        patch_grub_defaults_for_luks(dir.path(), luks_uuid).unwrap();

        let result = fs::read_to_string(etc.join("grub")).unwrap();

        assert!(
            result.contains(&format!("rd.luks.name={luks_uuid}=root")),
            "should contain rd.luks.name: {result}",
        );
        assert!(
            result.contains("rd.luks.options=discard"),
            "should contain rd.luks.options=discard: {result}",
        );
        assert!(
            !result.contains("console=ttyS0,115200n8"),
            "should have removed serial console: {result}",
        );
        assert!(
            result.contains("noresume"),
            "should preserve noresume: {result}",
        );
    }

    // r[verify installer.write.rebuild-boot-config+3]
    #[test]
    fn patch_grub_defaults_adds_cmdline_linux_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc/default");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("grub"),
            "GRUB_DEFAULT=0\n\
             GRUB_CMDLINE_LINUX_DEFAULT=\"noresume\"\n",
        )
        .unwrap();

        let luks_uuid = "aaaa-bbbb-cccc";
        patch_grub_defaults_for_luks(dir.path(), luks_uuid).unwrap();

        let result = fs::read_to_string(etc.join("grub")).unwrap();

        assert!(
            result.contains(&format!(
                "GRUB_CMDLINE_LINUX=\"rd.luks.name={luks_uuid}=root rd.luks.options=discard\""
            )),
            "should add GRUB_CMDLINE_LINUX when absent: {result}",
        );
    }
}
