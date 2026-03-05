use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use rand::{distr::slice::Choose, prelude::*};

use crate::config::DiskEncryption;

const KEYFILE_PATH: &str = "/etc/luks/keyfile";
const ROTATED_MARKER: &str = "/etc/luks/rotated";
const CRYPTTAB_PATH: &str = "/etc/crypttab";
const DRACUT_KEYFILE_CONF: &str = "/etc/dracut.conf.d/02-luks-keyfile.conf";
const PASSPHRASE_WORD_COUNT: usize = 6;

#[derive(Debug)]
pub struct EncryptionResult {
    pub recovery_passphrase: String,
}

/// Run the full encryption setup sequence on the target disk.
///
/// This must be called after writing the image and expanding partitions,
/// with the target's root filesystem already mounted at `mount_path`.
/// The LUKS volume should NOT be currently open when this is called;
/// the function operates directly on the raw partition for cryptsetup
/// commands and writes files into the mounted filesystem.
///
/// If `pre_generated_passphrase` is `Some`, that passphrase is enrolled
/// into the LUKS volume instead of generating a new one. This is the
/// normal path for the interactive TUI, which generates the passphrase at
/// confirmation time so the user can write it down before the destructive
/// write begins. In automatic mode, `None` is passed and a fresh
/// passphrase is generated here.
// r[impl installer.encryption.overview]
pub fn run_encryption_setup(
    target_device: &Path,
    disk_encryption: DiskEncryption,
    mount_path: &Path,
    pre_generated_passphrase: Option<&str>,
) -> Result<EncryptionResult> {
    if !disk_encryption.is_encrypted() {
        bail!("encryption setup called with disk_encryption=none");
    }

    let root_part = partition_path(target_device, 3)?;

    // r[impl installer.encryption.key-rotation]
    rotate_master_key(&root_part, mount_path)?;

    // r[impl installer.encryption.tpm-enroll]
    // r[impl installer.encryption.keyfile-enroll]
    enroll_unlock_mechanism(&root_part, disk_encryption, mount_path)?;

    // r[impl installer.encryption.recovery-passphrase+2]
    let passphrase = match pre_generated_passphrase {
        Some(p) => p.to_string(),
        None => generate_recovery_passphrase(),
    };
    enroll_recovery_passphrase(&root_part, &passphrase)?;

    // r[impl installer.encryption.wipe-empty-slot]
    wipe_empty_passphrase_slot(&root_part)?;

    // r[impl installer.encryption.configure-system]
    configure_installed_system(disk_encryption, mount_path)?;

    Ok(EncryptionResult {
        recovery_passphrase: passphrase,
    })
}

// r[impl installer.encryption.key-rotation]
fn rotate_master_key(root_part: &Path, mount_path: &Path) -> Result<()> {
    tracing::info!("rotating LUKS master key via online reencryption");

    let tmp_keyfile = create_empty_keyfile()?;

    let part_str = root_part.to_str().unwrap_or_default();
    let keyfile_str = tmp_keyfile.to_str().unwrap_or_default();

    run_command(
        "cryptsetup",
        &[
            "reencrypt",
            part_str,
            "--key-file",
            keyfile_str,
            "--batch-mode",
        ],
    )
    .context("rotating LUKS master key with cryptsetup reencrypt")?;

    let _ = fs::remove_file(&tmp_keyfile);

    let marker_path = mount_path.join(ROTATED_MARKER.strip_prefix('/').unwrap_or(ROTATED_MARKER));
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory for {}", marker_path.display()))?;
    }
    fs::write(&marker_path, "rotated by bes-installer\n")
        .with_context(|| format!("writing rotation marker {}", marker_path.display()))?;

    tracing::info!("LUKS master key rotated, marker written");
    Ok(())
}

fn enroll_unlock_mechanism(
    root_part: &Path,
    disk_encryption: DiskEncryption,
    mount_path: &Path,
) -> Result<()> {
    match disk_encryption {
        DiskEncryption::Tpm => enroll_tpm(root_part, mount_path)?,
        DiskEncryption::Keyfile => enroll_keyfile(root_part, mount_path)?,
        DiskEncryption::None => bail!("cannot enroll unlock mechanism for unencrypted disk"),
    }
    Ok(())
}

// r[impl installer.encryption.tpm-enroll]
fn enroll_tpm(root_part: &Path, mount_path: &Path) -> Result<()> {
    tracing::info!("enrolling TPM with PCR 1");

    let tmp_keyfile = create_empty_keyfile()?;

    let part_str = root_part.to_str().unwrap_or_default();
    let keyfile_str = tmp_keyfile.to_str().unwrap_or_default();

    run_command(
        "systemd-cryptenroll",
        &[
            part_str,
            "--unlock-key-file",
            keyfile_str,
            "--tpm2-device=auto",
            "--tpm2-pcrs=1",
        ],
    )
    .context("enrolling TPM via systemd-cryptenroll")?;

    let _ = fs::remove_file(&tmp_keyfile);

    let crypttab_path = mount_path.join(CRYPTTAB_PATH.strip_prefix('/').unwrap_or(CRYPTTAB_PATH));
    let crypttab_content = "# <name> <device>                    <keyfile>  <options>\n\
         root     /dev/disk/by-partlabel/root none       luks,discard,tpm2-device=auto,headless=true,timeout=30\n";
    fs::write(&crypttab_path, crypttab_content)
        .with_context(|| format!("writing crypttab at {}", crypttab_path.display()))?;

    tracing::info!("TPM enrolled and crypttab updated");
    Ok(())
}

// r[impl installer.encryption.keyfile-enroll]
fn enroll_keyfile(root_part: &Path, mount_path: &Path) -> Result<()> {
    tracing::info!("generating and enrolling random keyfile");

    let mut keyfile_data = vec![0u8; 4096];
    let mut urandom =
        fs::File::open("/dev/urandom").context("opening /dev/urandom for keyfile generation")?;
    urandom
        .read_exact(&mut keyfile_data)
        .context("reading random bytes for keyfile")?;

    let tmp_empty_keyfile = create_empty_keyfile()?;

    let tmp_new_keyfile = PathBuf::from("/tmp/bes-new-keyfile");
    fs::write(&tmp_new_keyfile, &keyfile_data).context("writing temporary new keyfile")?;
    fs::set_permissions(&tmp_new_keyfile, fs::Permissions::from_mode(0o400))
        .context("setting temporary keyfile permissions")?;

    let part_str = root_part.to_str().unwrap_or_default();
    let empty_str = tmp_empty_keyfile.to_str().unwrap_or_default();
    let new_str = tmp_new_keyfile.to_str().unwrap_or_default();

    run_command(
        "cryptsetup",
        &[
            "luksAddKey",
            part_str,
            new_str,
            "--key-file",
            empty_str,
            "--batch-mode",
        ],
    )
    .context("enrolling new keyfile via cryptsetup luksAddKey")?;

    let _ = fs::remove_file(&tmp_empty_keyfile);

    let installed_keyfile_path =
        mount_path.join(KEYFILE_PATH.strip_prefix('/').unwrap_or(KEYFILE_PATH));
    if let Some(parent) = installed_keyfile_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "creating directory for {}",
                installed_keyfile_path.display()
            )
        })?;
    }
    fs::write(&installed_keyfile_path, &keyfile_data)
        .with_context(|| format!("installing keyfile at {}", installed_keyfile_path.display()))?;
    fs::set_permissions(&installed_keyfile_path, fs::Permissions::from_mode(0o000)).with_context(
        || {
            format!(
                "setting permissions on {}",
                installed_keyfile_path.display()
            )
        },
    )?;

    let _ = fs::remove_file(&tmp_new_keyfile);

    let crypttab_path = mount_path.join(CRYPTTAB_PATH.strip_prefix('/').unwrap_or(CRYPTTAB_PATH));
    let crypttab_content = format!(
        "# <name> <device>                    <keyfile>         <options>\n\
         root     /dev/disk/by-partlabel/root {KEYFILE_PATH}  force,luks,discard,headless=true,timeout=30\n"
    );
    fs::write(&crypttab_path, &crypttab_content)
        .with_context(|| format!("writing crypttab at {}", crypttab_path.display()))?;

    let dracut_conf_path = mount_path.join(
        DRACUT_KEYFILE_CONF
            .strip_prefix('/')
            .unwrap_or(DRACUT_KEYFILE_CONF),
    );
    let dracut_content = format!("install_items+=\" {KEYFILE_PATH} \"\n");
    fs::write(&dracut_conf_path, &dracut_content)
        .with_context(|| format!("writing dracut config at {}", dracut_conf_path.display()))?;

    tracing::info!("keyfile enrolled, crypttab and dracut config updated");
    Ok(())
}

// r[impl installer.encryption.recovery-passphrase+2]
pub fn generate_recovery_passphrase() -> String {
    let mut rng = rand::rng();

    let wordlist = Choose::new(&diceware_wordlists::MINILOCK_WORDLIST).unwrap();
    let words: Vec<String> = wordlist
        .sample_iter(&mut rng)
        .filter(|w| w.len() >= 5 && w.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|w| w.to_ascii_lowercase())
        .take(PASSPHRASE_WORD_COUNT)
        .collect();

    words.join("-")
}

// r[impl installer.encryption.recovery-passphrase+2]
fn enroll_recovery_passphrase(root_part: &Path, passphrase: &str) -> Result<()> {
    tracing::info!("enrolling recovery passphrase");

    let tmp_keyfile = create_empty_keyfile()?;

    let part_str = root_part.to_str().unwrap_or_default();
    let keyfile_str = tmp_keyfile.to_str().unwrap_or_default();

    // Write the new passphrase to a temp file so we can pass it to
    // cryptsetup luksAddKey without interactive prompting.
    let tmp_passphrase = PathBuf::from("/tmp/bes-recovery-passphrase");
    fs::write(&tmp_passphrase, passphrase).context("writing temporary passphrase file")?;
    fs::set_permissions(&tmp_passphrase, fs::Permissions::from_mode(0o400))
        .context("setting passphrase file permissions")?;

    let passphrase_str = tmp_passphrase.to_str().unwrap_or_default();

    let result = run_command(
        "cryptsetup",
        &[
            "luksAddKey",
            part_str,
            passphrase_str,
            "--key-file",
            keyfile_str,
            "--batch-mode",
        ],
    )
    .context("enrolling recovery passphrase via cryptsetup luksAddKey");

    let _ = fs::remove_file(&tmp_keyfile);
    let _ = fs::remove_file(&tmp_passphrase);

    result?;

    tracing::info!("recovery passphrase enrolled");
    Ok(())
}

// r[impl installer.encryption.wipe-empty-slot]
fn wipe_empty_passphrase_slot(root_part: &Path) -> Result<()> {
    tracing::info!("finding and wiping the empty-passphrase key slot");

    let slot =
        find_slot_for_empty_keyfile(root_part).context("locating the empty-passphrase key slot")?;

    tracing::info!("empty passphrase is in slot {slot}");

    let part_str = root_part.to_str().unwrap_or_default();
    let slot_str = slot.to_string();

    run_command(
        "cryptsetup",
        &["luksKillSlot", part_str, &slot_str, "--batch-mode"],
    )
    .with_context(|| format!("wiping empty passphrase key slot {slot}"))?;

    tracing::info!("empty passphrase slot {slot} wiped");
    Ok(())
}

/// Probe each active LUKS key slot to find the one that unlocks with an
/// empty keyfile. Returns the slot number. After `cryptsetup reencrypt` the
/// original slot 0 is typically replaced by a new slot (often slot 1), so
/// we cannot assume the empty-passphrase key is always in slot 0.
fn find_slot_for_empty_keyfile(root_part: &Path) -> Result<u32> {
    let tmp_keyfile = create_empty_keyfile()?;
    let part_str = root_part.to_str().unwrap_or_default();
    let kf_str = tmp_keyfile.to_str().unwrap_or_default();

    // Parse active slot numbers from luksDump
    let dump_output = Command::new("cryptsetup")
        .args(["luksDump", part_str])
        .output()
        .context("running cryptsetup luksDump")?;
    if !dump_output.status.success() {
        let stderr = String::from_utf8_lossy(&dump_output.stderr);
        bail!("cryptsetup luksDump failed: {stderr}");
    }
    let dump = String::from_utf8_lossy(&dump_output.stdout);

    let mut slots = Vec::new();
    let mut in_keyslots = false;
    for line in dump.lines() {
        if line.starts_with("Keyslots:") {
            in_keyslots = true;
            continue;
        }
        if !in_keyslots {
            continue;
        }
        // A new top-level section starts at column 0 with a letter
        // (e.g. "Tokens:", "Digests:"). Stop there.
        if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        // Slot headers have exactly two leading spaces then a digit:
        //   "  0: luks2"
        // Detail lines start with a tab: "\tKey:  512 bits"
        if line.starts_with("  ") && !line.starts_with("   ") && !line.starts_with('\t') {
            let trimmed = line.trim();
            if let Some(colon) = trimmed.find(':')
                && let Ok(slot) = trimmed[..colon].trim().parse::<u32>()
            {
                slots.push(slot);
            }
        }
    }

    for slot in &slots {
        let ok = Command::new("cryptsetup")
            .args([
                "open",
                "--test-passphrase",
                "--key-slot",
                &slot.to_string(),
                "--key-file",
                kf_str,
                part_str,
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            let _ = fs::remove_file(&tmp_keyfile);
            return Ok(*slot);
        }
    }

    let _ = fs::remove_file(&tmp_keyfile);
    bail!("no LUKS key slot unlocks with the empty keyfile (checked slots: {slots:?})")
}

// r[impl installer.encryption.configure-system]
fn configure_installed_system(disk_encryption: DiskEncryption, mount_path: &Path) -> Result<()> {
    tracing::info!("rebuilding initramfs in installed system (encryption={disk_encryption})");

    // Bind-mount necessary virtual filesystems for chroot
    let proc_path = mount_path.join("proc");
    let sys_path = mount_path.join("sys");
    let dev_path = mount_path.join("dev");

    run_command(
        "mount",
        &["--bind", "/proc", proc_path.to_str().unwrap_or_default()],
    )
    .context("bind-mounting /proc into target")?;
    run_command(
        "mount",
        &["--bind", "/sys", sys_path.to_str().unwrap_or_default()],
    )
    .context("bind-mounting /sys into target")?;
    run_command(
        "mount",
        &["--bind", "/dev", dev_path.to_str().unwrap_or_default()],
    )
    .context("bind-mounting /dev into target")?;

    let mount_str = mount_path.to_str().unwrap_or_default();

    // Find the installed kernel version to rebuild its initramfs
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

    // Clean up bind mounts regardless of dracut result
    let _ = run_command("umount", &[dev_path.to_str().unwrap_or_default()]);
    let _ = run_command("umount", &[sys_path.to_str().unwrap_or_default()]);
    let _ = run_command("umount", &[proc_path.to_str().unwrap_or_default()]);

    dracut_result.context("rebuilding initramfs with dracut in chroot")?;

    tracing::info!("initramfs rebuilt successfully");
    Ok(())
}

fn partition_path(device: &Path, part_num: u32) -> Result<PathBuf> {
    let dev_str = device.to_str().unwrap_or_default();

    // NVMe and loop devices use "p" separator: /dev/nvme0n1p3, /dev/loop0p3
    // SCSI/SATA disks use no separator: /dev/sda3
    let path = if dev_str.ends_with(|c: char| c.is_ascii_digit()) {
        PathBuf::from(format!("{dev_str}p{part_num}"))
    } else {
        PathBuf::from(format!("{dev_str}{part_num}"))
    };

    Ok(path)
}

fn create_empty_keyfile() -> Result<PathBuf> {
    let path = PathBuf::from("/tmp/bes-empty-keyfile");
    fs::write(&path, b"").context("creating empty keyfile")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400))
        .context("setting keyfile permissions")?;
    Ok(path)
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

    // r[verify installer.encryption.recovery-passphrase+2]
    #[test]
    fn recovery_passphrase_has_correct_word_count() {
        let passphrase = generate_recovery_passphrase();
        let words: Vec<&str> = passphrase.split('-').collect();
        assert_eq!(
            words.len(),
            PASSPHRASE_WORD_COUNT,
            "passphrase should have {PASSPHRASE_WORD_COUNT} words, got: {passphrase}"
        );
    }

    // r[verify installer.encryption.recovery-passphrase+2]
    #[test]
    fn recovery_passphrase_words_are_nonempty() {
        let passphrase = generate_recovery_passphrase();
        for word in passphrase.split('-') {
            assert!(!word.is_empty(), "passphrase word should not be empty");
        }
    }

    // r[verify installer.encryption.recovery-passphrase+2]
    #[test]
    fn recovery_passphrases_are_unique() {
        let p1 = generate_recovery_passphrase();
        let p2 = generate_recovery_passphrase();
        // Technically could collide, but with 6 words from a large dictionary
        // the probability is vanishingly small.
        assert_ne!(p1, p2, "two consecutive passphrases should differ");
    }

    #[test]
    fn partition_path_scsi_disk() {
        let path = partition_path(Path::new("/dev/sda"), 3).unwrap();
        assert_eq!(path, PathBuf::from("/dev/sda3"));
    }

    #[test]
    fn partition_path_nvme() {
        let path = partition_path(Path::new("/dev/nvme0n1"), 3).unwrap();
        assert_eq!(path, PathBuf::from("/dev/nvme0n1p3"));
    }

    #[test]
    fn partition_path_loop() {
        let path = partition_path(Path::new("/dev/loop0"), 3).unwrap();
        assert_eq!(path, PathBuf::from("/dev/loop0p3"));
    }

    #[test]
    fn encryption_setup_rejects_none() {
        let result = run_encryption_setup(
            Path::new("/dev/sda"),
            DiskEncryption::None,
            Path::new("/mnt/target"),
            None,
        );
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("disk_encryption=none"),
            "error should mention none encryption: {err_msg}"
        );
    }
}
