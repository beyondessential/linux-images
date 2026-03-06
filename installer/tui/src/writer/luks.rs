use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub(crate) use crate::util::create_passphrase_keyfile;

pub(crate) const LUKS_NAME: &str = "bes-target-root";

// r[impl installer.write.luks-before-write+2]
pub fn format_luks_for_root(root_partition: &Path, passphrase: &str) -> Result<PathBuf> {
    tracing::info!(
        "formatting LUKS2 on {} with recovery passphrase",
        root_partition.display()
    );

    let keyfile = create_passphrase_keyfile(passphrase)?;

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

// r[impl installer.write.luks-before-write+2]
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

pub(crate) fn open_luks_root(root_partition: &Path, passphrase: &str) -> Result<PathBuf> {
    let keyfile = create_passphrase_keyfile(passphrase)?;

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
