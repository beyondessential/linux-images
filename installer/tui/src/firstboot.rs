// r[impl installer.firstboot.mount]
// r[impl installer.firstboot.hostname]
// r[impl installer.firstboot.tailscale-authkey]
// r[impl installer.firstboot.ssh-keys]
// r[impl installer.firstboot.tpm-disable]
// r[impl installer.firstboot.unmount]

use std::fs;
use std::os::unix::fs::{PermissionsExt, chown};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::{FirstbootConfig, Variant};

const MOUNT_BASE: &str = "/mnt/target";
const LUKS_NAME: &str = "bes-target-root";

pub struct MountedTarget {
    mount_path: PathBuf,
    luks_active: bool,
}

impl MountedTarget {
    pub fn path(&self) -> &Path {
        &self.mount_path
    }
}

pub fn mount_target(target_device: &Path, variant: Variant) -> Result<MountedTarget> {
    let root_part = partition_path(target_device, 3)?;

    let btrfs_dev = if variant == Variant::Metal {
        open_luks(&root_part)?
    } else {
        root_part
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
    .context("mounting target root")?;

    Ok(MountedTarget {
        mount_path,
        luks_active: variant == Variant::Metal,
    })
}

pub fn unmount_target(target: MountedTarget) -> Result<()> {
    run_command("umount", &[target.mount_path.to_str().unwrap_or_default()])
        .context("unmounting target root")?;

    if target.luks_active {
        close_luks()?;
    }

    let _ = fs::remove_dir(&target.mount_path);
    Ok(())
}

pub fn apply_firstboot(target: &MountedTarget, config: &FirstbootConfig) -> Result<()> {
    let root = target.path();

    if let Some(ref hostname) = config.hostname {
        apply_hostname(root, hostname)?;
    }

    if let Some(ref authkey) = config.tailscale_authkey {
        apply_tailscale_authkey(root, authkey)?;
    }

    if !config.ssh_authorized_keys.is_empty() {
        apply_ssh_keys(root, &config.ssh_authorized_keys)?;
    }

    Ok(())
}

pub fn apply_tpm_disable(target: &MountedTarget) -> Result<()> {
    let symlink = target
        .path()
        .join("etc/systemd/system/multi-user.target.wants/setup-tpm-unlock.service");

    if symlink.exists() || symlink.is_symlink() {
        fs::remove_file(&symlink).with_context(|| {
            format!(
                "removing setup-tpm-unlock.service symlink at {}",
                symlink.display()
            )
        })?;
        log::info!("removed setup-tpm-unlock.service enable symlink");
    } else {
        log::info!("setup-tpm-unlock.service symlink not present, nothing to remove");
    }

    Ok(())
}

fn apply_hostname(root: &Path, hostname: &str) -> Result<()> {
    let path = root.join("etc/hostname");
    fs::write(&path, format!("{hostname}\n"))
        .with_context(|| format!("writing hostname to {}", path.display()))?;

    let hosts_path = root.join("etc/hosts");
    if hosts_path.exists() {
        let contents = fs::read_to_string(&hosts_path).unwrap_or_default();
        if !contents.contains(hostname) {
            let mut new_contents = contents;
            new_contents.push_str(&format!("127.0.1.1 {hostname}\n"));
            fs::write(&hosts_path, new_contents)?;
        }
    }

    log::info!("set hostname to {hostname}");
    Ok(())
}

fn apply_tailscale_authkey(root: &Path, authkey: &str) -> Result<()> {
    let bes_dir = root.join("etc/bes");
    fs::create_dir_all(&bes_dir).context("creating /etc/bes")?;

    let key_path = bes_dir.join("tailscale-authkey");
    fs::write(&key_path, format!("{authkey}\n"))
        .with_context(|| format!("writing tailscale authkey to {}", key_path.display()))?;
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))
        .context("setting tailscale-authkey permissions")?;

    log::info!("wrote tailscale authkey");
    Ok(())
}

fn apply_ssh_keys(root: &Path, keys: &[String]) -> Result<()> {
    let ssh_dir = root.join("home/ubuntu/.ssh");
    fs::create_dir_all(&ssh_dir).context("creating .ssh directory")?;

    let ak_path = ssh_dir.join("authorized_keys");
    let mut contents = if ak_path.exists() {
        fs::read_to_string(&ak_path).unwrap_or_default()
    } else {
        String::new()
    };

    for key in keys {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            if !contents.is_empty() && !contents.ends_with('\n') {
                contents.push('\n');
            }
            contents.push_str(trimmed);
            contents.push('\n');
        }
    }

    fs::write(&ak_path, &contents).context("writing authorized_keys")?;

    fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700))
        .context("setting .ssh directory permissions")?;
    fs::set_permissions(&ak_path, fs::Permissions::from_mode(0o600))
        .context("setting authorized_keys permissions")?;

    let ubuntu_uid_gid = resolve_uid_gid_from_passwd(root, "ubuntu")?;
    chown(&ssh_dir, Some(ubuntu_uid_gid.0), Some(ubuntu_uid_gid.1))
        .context("chowning .ssh directory")?;
    chown(&ak_path, Some(ubuntu_uid_gid.0), Some(ubuntu_uid_gid.1))
        .context("chowning authorized_keys")?;

    log::info!("wrote {} SSH authorized key(s)", keys.len());
    Ok(())
}

fn resolve_uid_gid_from_passwd(root: &Path, username: &str) -> Result<(u32, u32)> {
    let passwd_path = root.join("etc/passwd");
    let contents = fs::read_to_string(&passwd_path).context("reading target /etc/passwd")?;

    for line in contents.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 4 && fields[0] == username {
            let uid: u32 = fields[2].parse().context("parsing uid")?;
            let gid: u32 = fields[3].parse().context("parsing gid")?;
            return Ok((uid, gid));
        }
    }

    bail!("user '{username}' not found in target /etc/passwd");
}

fn open_luks(partition: &Path) -> Result<PathBuf> {
    let keyfile = create_empty_keyfile()?;

    run_command(
        "cryptsetup",
        &[
            "open",
            partition.to_str().unwrap_or_default(),
            LUKS_NAME,
            "--key-file",
            keyfile.to_str().unwrap_or_default(),
        ],
    )
    .context("opening LUKS volume on target")?;

    let _ = fs::remove_file(&keyfile);

    Ok(PathBuf::from(format!("/dev/mapper/{LUKS_NAME}")))
}

fn close_luks() -> Result<()> {
    run_command("cryptsetup", &["close", LUKS_NAME]).context("closing LUKS volume")
}

fn create_empty_keyfile() -> Result<PathBuf> {
    let path = PathBuf::from("/tmp/bes-empty-keyfile");
    fs::write(&path, b"").context("creating empty keyfile")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400))
        .context("setting keyfile permissions")?;
    Ok(path)
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

fn run_command(program: &str, args: &[&str]) -> Result<()> {
    log::debug!("running: {program} {}", args.join(" "));

    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("spawning {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{program} failed (exit {}): {stderr}", output.status);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify test.static.cargo-test]

    #[test]
    fn partition_path_scsi_disk() {
        let p = partition_path(Path::new("/dev/sda"), 3).unwrap();
        assert_eq!(p, PathBuf::from("/dev/sda3"));
    }

    #[test]
    fn partition_path_nvme() {
        let p = partition_path(Path::new("/dev/nvme0n1"), 1).unwrap();
        assert_eq!(p, PathBuf::from("/dev/nvme0n1p1"));
    }

    #[test]
    fn partition_path_loop() {
        let p = partition_path(Path::new("/dev/loop0"), 2).unwrap();
        assert_eq!(p, PathBuf::from("/dev/loop0p2"));
    }

    #[test]
    fn resolve_uid_gid_from_passwd_contents() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("passwd"),
            "root:x:0:0:root:/root:/bin/bash\nubuntu:x:1000:1000:Ubuntu:/home/ubuntu:/bin/bash\n",
        )
        .unwrap();

        let (uid, gid) = resolve_uid_gid_from_passwd(dir.path(), "ubuntu").unwrap();
        assert_eq!(uid, 1000);
        assert_eq!(gid, 1000);
    }

    #[test]
    fn resolve_uid_gid_user_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("passwd"), "root:x:0:0:root:/root:/bin/bash\n").unwrap();

        assert!(resolve_uid_gid_from_passwd(dir.path(), "ubuntu").is_err());
    }
}
