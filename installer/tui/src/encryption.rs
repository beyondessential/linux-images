use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::{fs, io::Read, path::Path};

use anyhow::{Context, Result, bail};
use rand::{distr::slice::Choose, prelude::*};

use crate::config::DiskEncryption;
use crate::paths;
use crate::util::{create_passphrase_keyfile, partition_path, run_command};

const KEYFILE_PATH: &str = "/etc/luks/keyfile";
const CRYPTTAB_PATH: &str = "/etc/crypttab";
const DRACUT_KEYFILE_CONF: &str = "/etc/dracut.conf.d/02-luks-keyfile.conf";
const PASSPHRASE_WORD_COUNT: usize = 6;

/// Enroll the chosen unlock mechanism and write config files to the target.
///
/// This must be called after writing the image, expanding partitions, and
/// randomizing UUIDs, but **before** `rebuild_boot_config`. The target's
/// root filesystem must already be mounted at `mount_path`. The LUKS volume
/// should NOT be currently open when this is called; the function operates
/// directly on the raw partition for cryptsetup commands and writes files
/// into the mounted filesystem.
///
/// The recovery passphrase is already enrolled as the initial LUKS key
/// (it was used when formatting the volume in `format_luks_for_root`).
/// This function enrolls the operational unlock mechanism (TPM or keyfile)
/// and writes crypttab (and dracut keyfile config for keyfile mode).
/// The initramfs is NOT rebuilt here — that is handled by
/// `rebuild_boot_config`, which runs afterwards.
// r[impl installer.encryption.overview+3]
// r[impl installer.encryption.configure-system+2]
pub fn enroll_and_configure_encryption(
    target_device: &Path,
    disk_encryption: DiskEncryption,
    mount_path: &Path,
    recovery_passphrase: &str,
) -> Result<()> {
    if !disk_encryption.is_encrypted() {
        bail!("encryption setup called with disk_encryption=none");
    }

    let root_part = partition_path(target_device, 3)?;

    // r[impl installer.encryption.tpm-enroll+2]
    // r[impl installer.encryption.keyfile-enroll+2]
    enroll_unlock_mechanism(&root_part, disk_encryption, mount_path, recovery_passphrase)?;

    Ok(())
}

fn enroll_unlock_mechanism(
    root_part: &Path,
    disk_encryption: DiskEncryption,
    mount_path: &Path,
    recovery_passphrase: &str,
) -> Result<()> {
    match disk_encryption {
        DiskEncryption::Tpm => enroll_tpm(root_part, mount_path, recovery_passphrase)?,
        DiskEncryption::Keyfile => enroll_keyfile(root_part, mount_path, recovery_passphrase)?,
        DiskEncryption::None => bail!("cannot enroll unlock mechanism for unencrypted disk"),
    }
    Ok(())
}

// r[impl installer.encryption.tpm-enroll+2]
fn enroll_tpm(root_part: &Path, mount_path: &Path, recovery_passphrase: &str) -> Result<()> {
    tracing::info!("enrolling TPM with PCR 1");

    let tmp_keyfile = create_passphrase_keyfile(recovery_passphrase)?;

    let part_str = root_part.to_str().unwrap_or_default();
    let keyfile_str = tmp_keyfile.to_str().unwrap_or_default();

    run_command(
        paths::SYSTEMD_CRYPTENROLL,
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

// r[impl installer.encryption.keyfile-enroll+2]
fn enroll_keyfile(root_part: &Path, mount_path: &Path, recovery_passphrase: &str) -> Result<()> {
    tracing::info!("generating and enrolling random keyfile");

    let mut keyfile_data = vec![0u8; 4096];
    let mut urandom =
        fs::File::open("/dev/urandom").context("opening /dev/urandom for keyfile generation")?;
    urandom
        .read_exact(&mut keyfile_data)
        .context("reading random bytes for keyfile")?;

    let tmp_passphrase_keyfile = create_passphrase_keyfile(recovery_passphrase)?;

    let tmp_new_keyfile = PathBuf::from("/tmp/bes-new-keyfile");
    fs::write(&tmp_new_keyfile, &keyfile_data).context("writing temporary new keyfile")?;
    fs::set_permissions(&tmp_new_keyfile, fs::Permissions::from_mode(0o400))
        .context("setting temporary keyfile permissions")?;

    let part_str = root_part.to_str().unwrap_or_default();
    let passphrase_str = tmp_passphrase_keyfile.to_str().unwrap_or_default();
    let new_str = tmp_new_keyfile.to_str().unwrap_or_default();

    run_command(
        paths::CRYPTSETUP,
        &[
            "luksAddKey",
            part_str,
            new_str,
            "--key-file",
            passphrase_str,
            "--batch-mode",
        ],
    )
    .context("enrolling new keyfile via cryptsetup luksAddKey")?;

    let _ = fs::remove_file(&tmp_passphrase_keyfile);

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

// r[impl installer.encryption.recovery-passphrase+3]
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

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.encryption.recovery-passphrase+3]
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

    // r[verify installer.encryption.recovery-passphrase+3]
    #[test]
    fn recovery_passphrase_words_are_nonempty() {
        let passphrase = generate_recovery_passphrase();
        for word in passphrase.split('-') {
            assert!(!word.is_empty(), "passphrase word should not be empty");
        }
    }

    // r[verify installer.encryption.recovery-passphrase+3]
    #[test]
    fn recovery_passphrases_are_unique() {
        let p1 = generate_recovery_passphrase();
        let p2 = generate_recovery_passphrase();
        // Technically could collide, but with 6 words from a large dictionary
        // the probability is vanishingly small.
        assert_ne!(p1, p2, "two consecutive passphrases should differ");
    }

    #[test]
    fn encryption_setup_rejects_none() {
        let result: Result<()> = enroll_and_configure_encryption(
            Path::new("/dev/sda"),
            DiskEncryption::None,
            Path::new("/mnt/target"),
            "test-passphrase",
        );
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("disk_encryption=none"),
            "error should mention none encryption: {err_msg}"
        );
    }
}
