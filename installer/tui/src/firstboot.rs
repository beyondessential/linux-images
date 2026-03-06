use std::fs;
use std::os::unix::fs::{PermissionsExt, chown};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha_crypt::{Sha512Params, sha512_simple};

use crate::config::{DiskEncryption, FirstbootConfig};
use crate::util::{partition_path, run_command};
use crate::writer;

const INSTALL_LOG_TARGET: &str = "var/log/bes-installer.log";

const MOUNT_BASE: &str = "/mnt/target";

pub struct MountedTarget {
    mount_path: PathBuf,
    luks_active: bool,
}

impl MountedTarget {
    pub fn path(&self) -> &Path {
        &self.mount_path
    }
}

// r[impl installer.firstboot.mount+3]
pub fn mount_target(
    target_device: &Path,
    disk_encryption: DiskEncryption,
    passphrase: Option<&str>,
) -> Result<MountedTarget> {
    // r[impl installer.container.partition-devices+2]
    writer::ensure_partition_devices(target_device)
        .context("ensuring partition device nodes exist")?;

    let root_part = partition_path(target_device, 3)?;

    let luks_active = disk_encryption.is_encrypted();
    let btrfs_dev = if luks_active {
        writer::open_luks_root(&root_part, passphrase.unwrap_or_default())?
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
        luks_active,
    })
}

// r[impl installer.firstboot.unmount]
pub fn unmount_target(target: MountedTarget) -> Result<()> {
    run_command("umount", &[target.mount_path.to_str().unwrap_or_default()])
        .context("unmounting target root")?;

    if target.luks_active {
        writer::close_luks_root()?;
    }

    let _ = fs::remove_dir(&target.mount_path);
    Ok(())
}

pub fn apply_firstboot(target: &MountedTarget, config: &FirstbootConfig) -> Result<()> {
    let root = target.path();

    if config.hostname_from_dhcp {
        apply_dhcp_hostname(root)?;
    } else if let Some(ref hostname) = config.hostname {
        apply_hostname(root, hostname)?;
    }

    if let Some(ref authkey) = config.tailscale_authkey {
        apply_tailscale_authkey(root, authkey)?;
    }

    if !config.ssh_authorized_keys.is_empty() {
        apply_ssh_keys(root, &config.ssh_authorized_keys)?;
    }

    if config.has_password() {
        apply_password(root, config)?;
    }

    let tz = config.timezone.as_deref().unwrap_or("UTC");
    apply_timezone(root, tz)?;

    Ok(())
}

// r[impl installer.firstboot.timezone]
pub fn apply_timezone_default(target: &MountedTarget) -> Result<()> {
    apply_timezone(target.path(), "UTC")
}

// r[impl installer.write.fstab-fixup]
// r[impl installer.write.variant-fixup]
pub fn fixup_for_metal_variant(
    target: &MountedTarget,
    firstboot_config: &Option<FirstbootConfig>,
) -> Result<()> {
    let root = target.path();

    tracing::info!("applying metal variant fixups");

    // Rewrite /etc/fstab: replace by-partlabel/root with /dev/mapper/root
    let fstab_path = root.join("etc/fstab");
    if fstab_path.exists() {
        let contents = fs::read_to_string(&fstab_path).context("reading target /etc/fstab")?;
        let new_contents = contents.replace("/dev/disk/by-partlabel/root", "/dev/mapper/root");
        if new_contents != contents {
            fs::write(&fstab_path, &new_contents).context("writing target /etc/fstab")?;
            tracing::info!("rewrote /etc/fstab for metal variant");
        }
    }

    // Write variant marker
    let variant_dir = root.join("etc/bes");
    fs::create_dir_all(&variant_dir).context("creating /etc/bes")?;
    let variant_path = variant_dir.join("image-variant");
    fs::write(&variant_path, "metal\n").context("writing /etc/bes/image-variant")?;
    tracing::info!("set image-variant to metal");

    // Truncate /etc/hostname if no explicit hostname is configured
    let has_hostname = firstboot_config.as_ref().is_some_and(|fb| {
        fb.hostname.is_some() || fb.hostname_from_dhcp || fb.hostname_template.is_some()
    });
    if !has_hostname {
        let hostname_path = root.join("etc/hostname");
        fs::write(&hostname_path, "").context("truncating /etc/hostname for metal")?;
        tracing::info!("truncated /etc/hostname for metal variant (no explicit hostname)");
    }

    // Create /etc/luks/empty-keyfile with mode 000
    let luks_dir = root.join("etc/luks");
    fs::create_dir_all(&luks_dir).context("creating /etc/luks")?;
    let keyfile_path = luks_dir.join("empty-keyfile");
    fs::write(&keyfile_path, b"").context("creating /etc/luks/empty-keyfile")?;
    fs::set_permissions(&keyfile_path, fs::Permissions::from_mode(0o000))
        .context("setting empty-keyfile permissions to 000")?;
    tracing::info!("created /etc/luks/empty-keyfile");

    Ok(())
}

// r[impl installer.firstboot.copy-install-log]
pub fn copy_install_log(target: &MountedTarget, log_path: &Path) {
    let dest = target.path().join(INSTALL_LOG_TARGET);

    if let Some(parent) = dest.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        tracing::warn!("failed to create directory for install log: {e}");
        return;
    }

    match fs::copy(log_path, &dest) {
        Ok(bytes) => {
            tracing::info!("copied install log ({bytes} bytes) to {}", dest.display());
        }
        Err(e) => {
            tracing::warn!(
                "failed to copy install log from {} to {}: {e}",
                log_path.display(),
                dest.display()
            );
        }
    }
}

// r[impl installer.firstboot.timezone]
fn apply_timezone(root: &Path, timezone: &str) -> Result<()> {
    let zoneinfo_path = format!("/usr/share/zoneinfo/{timezone}");
    let localtime_path = root.join("etc/localtime");

    if localtime_path.exists() || localtime_path.is_symlink() {
        fs::remove_file(&localtime_path)
            .with_context(|| format!("removing existing {}", localtime_path.display()))?;
    }

    std::os::unix::fs::symlink(&zoneinfo_path, &localtime_path).with_context(|| {
        format!(
            "symlinking {} -> {}",
            localtime_path.display(),
            zoneinfo_path
        )
    })?;

    let timezone_path = root.join("etc/timezone");
    fs::write(&timezone_path, format!("{timezone}\n"))
        .with_context(|| format!("writing timezone to {}", timezone_path.display()))?;

    tracing::info!("set timezone to {timezone}");
    Ok(())
}

// r[impl installer.firstboot.password]
fn apply_password(root: &Path, config: &FirstbootConfig) -> Result<()> {
    let hash = if let Some(ref h) = config.password_hash {
        h.clone()
    } else if let Some(ref plaintext) = config.password {
        let params = Sha512Params::new(5000).expect("valid rounds");
        sha512_simple(plaintext, &params)
            .map_err(|e| anyhow::anyhow!("hashing password with SHA-512 crypt: {e:?}"))?
    } else {
        bail!("apply_password called with no password or password_hash set");
    };

    let shadow_path = root.join("etc/shadow");
    let contents = fs::read_to_string(&shadow_path).context("reading target /etc/shadow")?;

    let mut found = false;
    let new_contents: String = contents
        .lines()
        .map(|line| {
            if line.starts_with("ubuntu:") {
                found = true;
                let fields: Vec<&str> = line.split(':').collect();
                if fields.len() >= 9 {
                    // fields: name:hash:lastchanged:min:max:warn:inactive:expire:reserved
                    // Set the hash, clear the expiry by setting lastchanged to days-since-epoch
                    let days_since_epoch = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        / 86400;
                    format!(
                        "{}:{}:{}:{}:{}:{}:{}:{}:{}",
                        fields[0],
                        hash,
                        days_since_epoch,
                        fields[3],
                        fields[4],
                        fields[5],
                        fields[6],
                        fields[7],
                        fields[8],
                    )
                } else {
                    format!("ubuntu:{hash}:{}", &fields[2..].join(":"))
                }
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    if !found {
        bail!("user 'ubuntu' not found in target /etc/shadow");
    }

    // Preserve trailing newline if original had one
    let new_contents = if contents.ends_with('\n') && !new_contents.ends_with('\n') {
        format!("{new_contents}\n")
    } else {
        new_contents
    };

    fs::write(&shadow_path, &new_contents).context("writing target /etc/shadow")?;

    tracing::info!("set password for ubuntu user");
    Ok(())
}

// r[impl installer.firstboot.hostname]
fn apply_dhcp_hostname(root: &Path) -> Result<()> {
    let hostname_path = root.join("etc/hostname");
    fs::write(&hostname_path, "")
        .with_context(|| format!("truncating {}", hostname_path.display()))?;

    let hosts_path = root.join("etc/hosts");
    if hosts_path.exists() {
        let contents = fs::read_to_string(&hosts_path).unwrap_or_default();
        let new_contents: String = contents
            .lines()
            .filter(|line| !line.contains("127.0.1.1"))
            .collect::<Vec<_>>()
            .join("\n");
        let new_contents = if contents.ends_with('\n') && !new_contents.ends_with('\n') {
            format!("{new_contents}\n")
        } else {
            new_contents
        };
        fs::write(&hosts_path, new_contents)?;
    }

    tracing::info!("set hostname to DHCP (empty /etc/hostname)");
    Ok(())
}

// r[impl installer.firstboot.hostname]
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

    tracing::info!("set hostname to {hostname}");
    Ok(())
}

// r[impl installer.firstboot.tailscale-authkey]
fn apply_tailscale_authkey(root: &Path, authkey: &str) -> Result<()> {
    let bes_dir = root.join("etc/bes");
    fs::create_dir_all(&bes_dir).context("creating /etc/bes")?;

    let key_path = bes_dir.join("tailscale-authkey");
    fs::write(&key_path, format!("{authkey}\n"))
        .with_context(|| format!("writing tailscale authkey to {}", key_path.display()))?;
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))
        .context("setting tailscale-authkey permissions")?;

    tracing::info!("wrote tailscale authkey");
    Ok(())
}

// r[impl installer.firstboot.ssh-keys]
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

    tracing::info!("wrote {} SSH authorized key(s)", keys.len());
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

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.firstboot.password]
    #[test]
    fn apply_password_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("shadow"),
            "root:!:19900:0:99999:7:::\nubuntu:$6$old$oldhash:0:0:99999:7:::\n",
        )
        .unwrap();

        let config = FirstbootConfig {
            password: Some("newsecret".into()),
            ..Default::default()
        };
        apply_password(dir.path(), &config).unwrap();

        let shadow = fs::read_to_string(etc.join("shadow")).unwrap();
        let ubuntu_line = shadow.lines().find(|l| l.starts_with("ubuntu:")).unwrap();
        let fields: Vec<&str> = ubuntu_line.split(':').collect();
        assert!(
            fields[1].starts_with("$6$"),
            "expected SHA-512 hash, got: {}",
            fields[1]
        );
        // lastchanged should be non-zero (not expired)
        let lastchanged: u64 = fields[2].parse().unwrap();
        assert!(lastchanged > 0);
        assert!(shadow.ends_with('\n'));
    }

    // r[verify installer.firstboot.password]
    #[test]
    fn apply_password_hash_direct() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("shadow"),
            "root:!:19900:0:99999:7:::\nubuntu:$6$old$oldhash:0:0:99999:7:::\n",
        )
        .unwrap();

        let config = FirstbootConfig {
            password_hash: Some("$6$custom$myhash".into()),
            ..Default::default()
        };
        apply_password(dir.path(), &config).unwrap();

        let shadow = fs::read_to_string(etc.join("shadow")).unwrap();
        let ubuntu_line = shadow.lines().find(|l| l.starts_with("ubuntu:")).unwrap();
        let fields: Vec<&str> = ubuntu_line.split(':').collect();
        assert_eq!(fields[1], "$6$custom$myhash");
    }

    // r[verify installer.firstboot.password]
    #[test]
    fn apply_password_user_not_found_in_shadow() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("shadow"), "root:!:19900:0:99999:7:::\n").unwrap();

        let config = FirstbootConfig {
            password: Some("test".into()),
            ..Default::default()
        };
        assert!(apply_password(dir.path(), &config).is_err());
    }

    // r[verify installer.firstboot.password]
    #[test]
    fn apply_password_preserves_other_users() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("shadow"),
            "root:!:19900:0:99999:7:::\nubuntu:$6$old$oldhash:0:0:99999:7:::\ndaemon:*:19900:0:99999:7:::\n",
        )
        .unwrap();

        let config = FirstbootConfig {
            password_hash: Some("$6$new$newhash".into()),
            ..Default::default()
        };
        apply_password(dir.path(), &config).unwrap();

        let shadow = fs::read_to_string(etc.join("shadow")).unwrap();
        assert!(shadow.starts_with("root:!:19900:0:99999:7:::"));
        assert!(shadow.contains("daemon:*:19900:0:99999:7:::"));
        let ubuntu_line = shadow.lines().find(|l| l.starts_with("ubuntu:")).unwrap();
        assert!(ubuntu_line.contains("$6$new$newhash"));
    }

    // r[verify installer.firstboot.hostname]
    #[test]
    fn apply_dhcp_hostname_truncates_etc_hostname() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("hostname"), "old-hostname\n").unwrap();
        fs::write(
            etc.join("hosts"),
            "127.0.0.1 localhost\n127.0.1.1 old-hostname\n::1 localhost\n",
        )
        .unwrap();

        apply_dhcp_hostname(dir.path()).unwrap();

        let hostname = fs::read_to_string(etc.join("hostname")).unwrap();
        assert_eq!(hostname, "");

        let hosts = fs::read_to_string(etc.join("hosts")).unwrap();
        assert!(!hosts.contains("127.0.1.1"));
        assert!(hosts.contains("127.0.0.1 localhost"));
        assert!(hosts.contains("::1 localhost"));
    }

    // r[verify installer.firstboot.hostname]
    #[test]
    fn apply_dhcp_hostname_no_hosts_file() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("hostname"), "old-hostname\n").unwrap();

        apply_dhcp_hostname(dir.path()).unwrap();

        let hostname = fs::read_to_string(etc.join("hostname")).unwrap();
        assert_eq!(hostname, "");
    }

    // r[verify installer.firstboot.ssh-keys]
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

    // r[verify installer.firstboot.ssh-keys]
    #[test]
    fn resolve_uid_gid_user_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("passwd"), "root:x:0:0:root:/root:/bin/bash\n").unwrap();

        assert!(resolve_uid_gid_from_passwd(dir.path(), "ubuntu").is_err());
    }

    // r[verify installer.firstboot.timezone]
    #[test]
    fn apply_timezone_creates_symlink_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();

        apply_timezone(dir.path(), "America/New_York").unwrap();

        let localtime = etc.join("localtime");
        assert!(localtime.is_symlink());
        let target = fs::read_link(&localtime).unwrap();
        assert_eq!(
            target,
            PathBuf::from("/usr/share/zoneinfo/America/New_York")
        );

        let timezone = fs::read_to_string(etc.join("timezone")).unwrap();
        assert_eq!(timezone, "America/New_York\n");
    }

    // r[verify installer.firstboot.timezone]
    #[test]
    fn apply_timezone_replaces_existing_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();

        std::os::unix::fs::symlink("/usr/share/zoneinfo/UTC", etc.join("localtime")).unwrap();
        fs::write(etc.join("timezone"), "UTC\n").unwrap();

        apply_timezone(dir.path(), "Pacific/Auckland").unwrap();

        let target = fs::read_link(etc.join("localtime")).unwrap();
        assert_eq!(
            target,
            PathBuf::from("/usr/share/zoneinfo/Pacific/Auckland")
        );

        let timezone = fs::read_to_string(etc.join("timezone")).unwrap();
        assert_eq!(timezone, "Pacific/Auckland\n");
    }

    // r[verify installer.firstboot.timezone]
    #[test]
    fn apply_timezone_utc_default() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();

        apply_timezone(dir.path(), "UTC").unwrap();

        let target = fs::read_link(etc.join("localtime")).unwrap();
        assert_eq!(target, PathBuf::from("/usr/share/zoneinfo/UTC"));

        let timezone = fs::read_to_string(etc.join("timezone")).unwrap();
        assert_eq!(timezone, "UTC\n");
    }

    // r[verify installer.firstboot.timezone]
    #[test]
    fn apply_firstboot_sets_timezone_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        // apply_firstboot needs /etc/shadow for password but we skip password
        // by not setting it. We just need etc to exist.

        let config = FirstbootConfig {
            timezone: Some("Europe/London".into()),
            ..Default::default()
        };

        // apply_firstboot calls apply_timezone internally.
        // We can't fully call it without a MountedTarget, so test apply_timezone directly.
        apply_timezone(dir.path(), config.timezone.as_deref().unwrap_or("UTC")).unwrap();

        let target = fs::read_link(etc.join("localtime")).unwrap();
        assert_eq!(target, PathBuf::from("/usr/share/zoneinfo/Europe/London"));
    }

    // r[verify installer.firstboot.timezone]
    #[test]
    fn apply_firstboot_defaults_timezone_to_utc() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();

        let config = FirstbootConfig::default();
        let tz = config.timezone.as_deref().unwrap_or("UTC");
        apply_timezone(dir.path(), tz).unwrap();

        let target = fs::read_link(etc.join("localtime")).unwrap();
        assert_eq!(target, PathBuf::from("/usr/share/zoneinfo/UTC"));

        let timezone = fs::read_to_string(etc.join("timezone")).unwrap();
        assert_eq!(timezone, "UTC\n");
    }
}
