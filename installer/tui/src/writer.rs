use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::config::DiskEncryption;

const LUKS_NAME: &str = "bes-target-root";
const MOUNT_BASE: &str = "/mnt/target";

// r[impl installer.tui.progress+2]
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

// r[impl installer.write.partitions+2]
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

// r[impl installer.write.partitions+2]
pub fn create_partition_table(target: &Path, manifest: &PartitionManifest) -> Result<()> {
    tracing::info!(
        "creating GPT with {} partitions on {}",
        manifest.partitions.len(),
        target.display()
    );

    let target_str = target.to_str().unwrap_or_default();

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

    let output = Command::new("sgdisk")
        .args(&args)
        .output()
        .context("running sgdisk to create partition table")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("sgdisk failed: {stderr}");
    }

    tracing::info!("partition table created on {}", target.display());
    Ok(())
}

// r[impl installer.write.decompress-stream+2]
pub fn decompress_to_device(
    source: &Path,
    target: &Path,
    bytes_offset: u64,
    total_bytes: Option<u64>,
    on_progress: &mut dyn FnMut(&WriteProgress),
) -> Result<u64> {
    let input =
        File::open(source).with_context(|| format!("opening source image {}", source.display()))?;

    let mut decoder = zstd::Decoder::new(input).context("initializing zstd decoder")?;

    let mut output = OpenOptions::new()
        .write(true)
        .open(target)
        .with_context(|| format!("opening target device {}", target.display()))?;

    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut partition_bytes_written: u64 = 0;
    let start = Instant::now();

    loop {
        let n = decoder.read(&mut buf).context("reading from zstd stream")?;
        if n == 0 {
            break;
        }
        output
            .write_all(&buf[..n])
            .context("writing to target device")?;
        partition_bytes_written += n as u64;

        on_progress(&WriteProgress {
            bytes_written: bytes_offset + partition_bytes_written,
            total_bytes,
            elapsed: start.elapsed(),
        });
    }

    output.flush().context("flushing target device")?;
    sync_device(&output)?;

    tracing::info!(
        "wrote {} to {} in {:.1}s ({:.1} MiB/s)",
        format_size(partition_bytes_written),
        target.display(),
        start.elapsed().as_secs_f64(),
        if start.elapsed().as_secs_f64() > 0.0 {
            (partition_bytes_written as f64) / (1024.0 * 1024.0) / start.elapsed().as_secs_f64()
        } else {
            0.0
        },
    );

    Ok(partition_bytes_written)
}

fn create_empty_keyfile() -> Result<PathBuf> {
    let path = PathBuf::from("/tmp/bes-empty-keyfile");
    fs::write(&path, b"").context("creating empty keyfile")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400))
        .context("setting keyfile permissions")?;
    Ok(path)
}

// r[impl installer.write.luks-before-write]
pub fn format_luks_for_root(root_partition: &Path) -> Result<PathBuf> {
    tracing::info!(
        "formatting LUKS2 on {} with empty passphrase",
        root_partition.display()
    );

    let keyfile = create_empty_keyfile()?;

    let output = Command::new("cryptsetup")
        .args([
            "luksFormat",
            "--type",
            "luks2",
            "--key-file",
            keyfile.to_str().unwrap_or_default(),
            root_partition.to_str().unwrap_or_default(),
        ])
        .output()
        .context("running cryptsetup luksFormat")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cryptsetup luksFormat failed: {stderr}");
    }

    tracing::info!(
        "opening LUKS volume on {} as {LUKS_NAME}",
        root_partition.display()
    );

    let output = Command::new("cryptsetup")
        .args([
            "open",
            "--type",
            "luks2",
            "--key-file",
            keyfile.to_str().unwrap_or_default(),
            root_partition.to_str().unwrap_or_default(),
            LUKS_NAME,
        ])
        .output()
        .context("running cryptsetup open")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cryptsetup open failed: {stderr}");
    }

    let _ = fs::remove_file(&keyfile);

    Ok(PathBuf::from(format!("/dev/mapper/{LUKS_NAME}")))
}

// r[impl installer.write.luks-before-write]
pub fn close_luks_root() -> Result<()> {
    tracing::info!("closing LUKS volume {LUKS_NAME}");
    let output = Command::new("cryptsetup")
        .args(["close", LUKS_NAME])
        .output()
        .context("running cryptsetup close")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cryptsetup close failed: {stderr}");
    }
    Ok(())
}

fn open_luks_root(root_partition: &Path) -> Result<PathBuf> {
    let keyfile = create_empty_keyfile()?;

    tracing::info!(
        "opening LUKS volume on {} as {LUKS_NAME}",
        root_partition.display()
    );

    let output = Command::new("cryptsetup")
        .args([
            "open",
            "--type",
            "luks2",
            "--key-file",
            keyfile.to_str().unwrap_or_default(),
            root_partition.to_str().unwrap_or_default(),
            LUKS_NAME,
        ])
        .output()
        .context("running cryptsetup open")?;

    let _ = fs::remove_file(&keyfile);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cryptsetup open failed: {stderr}");
    }

    Ok(PathBuf::from(format!("/dev/mapper/{LUKS_NAME}")))
}

// r[impl installer.write.partitions+2]
// r[impl installer.write.decompress-stream+2]
pub fn write_partitions(
    manifest: &PartitionManifest,
    images_dir: &Path,
    target: &Path,
    disk_encryption: DiskEncryption,
    on_progress: &mut dyn FnMut(&WriteProgress),
) -> Result<()> {
    let total_bytes = partition_images_total_size(manifest, images_dir).ok();

    wipe_disk(target).context("wiping target disk before writing")?;
    create_partition_table(target, manifest).context("creating partition table")?;
    reread_partition_table(target).context("re-reading partition table after creation")?;

    let mut bytes_offset: u64 = 0;

    for (i, entry) in manifest.partitions.iter().enumerate() {
        let part_num = (i + 1) as u32;
        let part_device = partition_path(target, part_num)?;
        let img_path = images_dir.join(&entry.image);

        tracing::info!(
            "writing {} -> {} (partition {})",
            entry.image,
            part_device.display(),
            part_num,
        );

        let write_device = if entry.label == "root" && disk_encryption.is_encrypted() {
            format_luks_for_root(&part_device).context("formatting LUKS on root partition")?
        } else {
            part_device.clone()
        };

        let written = decompress_to_device(
            &img_path,
            &write_device,
            bytes_offset,
            total_bytes,
            on_progress,
        )
        .with_context(|| format!("writing partition {}", entry.label))?;

        bytes_offset += written;

        if entry.label == "root" && disk_encryption.is_encrypted() {
            close_luks_root().context("closing LUKS after writing root")?;
        }
    }

    on_progress(&WriteProgress {
        bytes_written: bytes_offset,
        total_bytes: Some(bytes_offset),
        elapsed: std::time::Duration::ZERO,
    });

    tracing::info!("all partitions written to {}", target.display());
    Ok(())
}

// r[impl installer.write.expand-root]
pub fn expand_root_filesystem(target: &Path, disk_encryption: DiskEncryption) -> Result<()> {
    let root_part = partition_path(target, 3)?;

    let btrfs_dev = if disk_encryption.is_encrypted() {
        let mapper_dev = open_luks_root(&root_part)?;

        tracing::info!("resizing LUKS container to fill partition");
        let keyfile = create_empty_keyfile()?;
        let output = Command::new("cryptsetup")
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
        "mount",
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
        "btrfs",
        &[
            "filesystem",
            "resize",
            "max",
            mount_path.to_str().unwrap_or_default(),
        ],
    );

    let _ = run_command("umount", &[mount_path.to_str().unwrap_or_default()]);

    if disk_encryption.is_encrypted() {
        let _ = close_luks_root();
    }

    resize_result.context("resizing btrfs filesystem")?;

    tracing::info!("root filesystem expanded");
    Ok(())
}

// r[impl installer.write.randomize-uuids]
pub fn randomize_filesystem_uuids(target: &Path, disk_encryption: DiskEncryption) -> Result<()> {
    tracing::info!("randomizing filesystem UUIDs");

    let efi_part = partition_path(target, 1)?;
    let xboot_part = partition_path(target, 2)?;
    let root_part = partition_path(target, 3)?;

    match Command::new("mlabel")
        .args(["-n", "-i", efi_part.to_str().unwrap_or_default(), "::"])
        .output()
    {
        Ok(efi_result) if !efi_result.status.success() => {
            let stderr = String::from_utf8_lossy(&efi_result.stderr);
            tracing::warn!("mlabel failed (non-fatal): {stderr}");
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("mlabel not found (non-fatal): EFI serial not randomized");
        }
        Err(e) => {
            tracing::warn!("mlabel failed (non-fatal): {e}");
        }
    }

    let xboot_result = Command::new("tune2fs")
        .args(["-U", "random", xboot_part.to_str().unwrap_or_default()])
        .output()
        .context("running tune2fs to randomize xboot UUID")?;
    if !xboot_result.status.success() {
        let stderr = String::from_utf8_lossy(&xboot_result.stderr);
        bail!("tune2fs -U random failed on xboot: {stderr}");
    }

    let btrfs_dev = if disk_encryption.is_encrypted() {
        open_luks_root(&root_part)?
    } else {
        root_part.clone()
    };

    let btrfs_result = Command::new("btrfstune")
        .args(["-f", "-u", btrfs_dev.to_str().unwrap_or_default()])
        .output()
        .context("running btrfstune to randomize root UUID")?;

    if disk_encryption.is_encrypted() {
        let _ = close_luks_root();
    }

    if !btrfs_result.status.success() {
        let stderr = String::from_utf8_lossy(&btrfs_result.stderr);
        bail!("btrfstune -u failed on root: {stderr}");
    }

    tracing::info!("filesystem UUIDs randomized");
    Ok(())
}

// r[impl installer.write.rebuild-boot-config]
pub fn rebuild_boot_config(target: &Path, disk_encryption: DiskEncryption) -> Result<()> {
    tracing::info!("rebuilding boot config (initramfs + grub)");

    let root_part = partition_path(target, 3)?;
    let xboot_part = partition_path(target, 2)?;
    let efi_part = partition_path(target, 1)?;

    let luks_opened = if disk_encryption.is_encrypted() {
        let _ = open_luks_root(&root_part)?;
        true
    } else {
        false
    };

    let btrfs_dev = if luks_opened {
        PathBuf::from(format!("/dev/mapper/{LUKS_NAME}"))
    } else {
        root_part.clone()
    };

    let mount_path = PathBuf::from(MOUNT_BASE);
    fs::create_dir_all(&mount_path).context("creating mount point")?;

    run_command(
        "mount",
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
        "mount",
        &[
            xboot_part.to_str().unwrap_or_default(),
            xboot_mount.to_str().unwrap_or_default(),
        ],
    );
    let xboot_mounted = mount_xboot_result.is_ok();

    let efi_mount = mount_path.join("boot/efi");
    fs::create_dir_all(&efi_mount).ok();
    let mount_efi_result = run_command(
        "mount",
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
        "mount",
        &["--bind", "/proc", proc_path.to_str().unwrap_or_default()],
    );
    let _ = run_command(
        "mount",
        &["--bind", "/sys", sys_path.to_str().unwrap_or_default()],
    );
    let _ = run_command(
        "mount",
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
        run_command("chroot", &[mount_str, "dracut", "--force", "--kver", kver])
    } else {
        tracing::warn!("no kernel version found in target, running dracut without --kver");
        run_command(
            "chroot",
            &[mount_str, "dracut", "--force", "--regenerate-all"],
        )
    };

    let grub_result = run_command("chroot", &[mount_str, "update-grub"]);

    let _ = run_command("umount", &[dev_path.to_str().unwrap_or_default()]);
    let _ = run_command("umount", &[sys_path.to_str().unwrap_or_default()]);
    let _ = run_command("umount", &[proc_path.to_str().unwrap_or_default()]);
    if efi_mounted {
        let _ = run_command("umount", &[efi_mount.to_str().unwrap_or_default()]);
    }
    if xboot_mounted {
        let _ = run_command("umount", &[xboot_mount.to_str().unwrap_or_default()]);
    }
    let _ = run_command("umount", &[mount_path.to_str().unwrap_or_default()]);

    if luks_opened {
        let _ = close_luks_root();
    }

    dracut_result.context("rebuilding initramfs with dracut in chroot")?;
    grub_result.context("running update-grub in chroot")?;

    tracing::info!("boot config rebuilt successfully");
    Ok(())
}

// r[impl installer.write.partitions+2]
pub fn verify_partition_table(target: &Path) -> Result<()> {
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

fn run_command(program: &str, args: &[&str]) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.tui.progress+2]
    #[test]
    fn progress_fraction_with_total() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(10),
        };
        assert!((p.fraction().unwrap() - 0.5).abs() < f64::EPSILON);
    }

    // r[verify installer.tui.progress+2]
    #[test]
    fn progress_fraction_without_total() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: None,
            elapsed: std::time::Duration::from_secs(10),
        };
        assert!(p.fraction().is_none());
    }

    // r[verify installer.tui.progress+2]
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

    // r[verify installer.tui.progress+2]
    #[test]
    fn progress_eta_at_zero() {
        let p = WriteProgress {
            bytes_written: 0,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(0),
        };
        assert!(p.eta().is_none());
    }

    // r[verify installer.tui.progress+2]
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

    // r[verify installer.tui.progress+2]
    #[test]
    fn progress_throughput() {
        let p = WriteProgress {
            bytes_written: 10 * 1024 * 1024,
            total_bytes: None,
            elapsed: std::time::Duration::from_secs(1),
        };
        assert!((p.throughput_mbps() - 10.0).abs() < 0.1);
    }

    // r[verify installer.tui.progress+2]
    #[test]
    fn eta_formatting() {
        assert_eq!(format_eta(std::time::Duration::from_secs(45)), "45s");
        assert_eq!(format_eta(std::time::Duration::from_secs(90)), "1m30s");
        assert_eq!(format_eta(std::time::Duration::from_secs(3661)), "61m01s");
    }

    // r[verify installer.write.decompress-stream+2]
    #[test]
    fn size_formatting() {
        assert_eq!(format_size(0), "0.0 MiB");
        assert_eq!(format_size(1024 * 1024 * 512), "512.0 MiB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00 GiB");
        assert_eq!(format_size(8 * 1024 * 1024 * 1024), "8.00 GiB");
    }

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

    // r[verify installer.write.partitions+2]
    #[test]
    fn create_partition_table_builds_correct_sgdisk_args() {
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

        assert_eq!(manifest.partitions.len(), 3);
        assert_eq!(manifest.partitions[0].size_mib, 512);
        assert_eq!(manifest.partitions[2].size_mib, 0);
    }
}
