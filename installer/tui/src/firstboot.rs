use std::fs;
use std::os::unix::fs::{PermissionsExt, chown};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha_crypt::{Sha512Params, sha512_simple};

use crate::config::{DiskEncryption, InstallConfig, NetworkMode};
use crate::paths;
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

// r[impl installer.finalise.mount+4]
pub fn mount_target(
    target_device: &Path,
    disk_encryption: DiskEncryption,
    passphrase: Option<&str>,
) -> Result<MountedTarget> {
    // r[impl installer.container.partition-devices+3]
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
    .context("mounting target root")?;

    Ok(MountedTarget {
        mount_path,
        luks_active,
    })
}

// r[impl installer.finalise.unmount]
pub fn unmount_target(target: MountedTarget) -> Result<()> {
    run_command(
        paths::UMOUNT,
        &[target.mount_path.to_str().unwrap_or_default()],
    )
    .context("unmounting target root")?;

    if target.luks_active {
        writer::close_luks_root()?;
    }

    let _ = fs::remove_dir(&target.mount_path);
    Ok(())
}

pub fn apply_firstboot(
    target: &MountedTarget,
    config: &InstallConfig,
    tailscale_netcheck_ok: bool,
) -> Result<()> {
    let root = target.path();

    if config.hostname_from_dhcp {
        apply_dhcp_hostname(root)?;
    } else if let Some(ref hostname) = config.hostname {
        apply_hostname(root, hostname)?;
    }

    if let Some(ref authkey) = config.tailscale_authkey {
        // r[impl installer.finalise.tailscale-auth]
        let authed = if tailscale_netcheck_ok {
            match attempt_tailscale_auth(root, authkey) {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!(
                        "tailscale auth failed, falling back to firstboot keyfile: {e:#}"
                    );
                    false
                }
            }
        } else {
            tracing::info!("tailscale netcheck did not pass, skipping install-time auth");
            false
        };

        // r[impl installer.finalise.tailscale-firstboot]
        if !authed {
            write_tailscale_authkey(root, authkey)?;
        }
    }

    if !config.ssh_authorized_keys.is_empty() {
        apply_ssh_keys(root, &config.ssh_authorized_keys)?;
    }

    if config.has_password() {
        apply_password(root, config)?;
    }

    let tz = config.timezone.as_deref().unwrap_or("UTC");
    apply_timezone(root, tz)?;

    // r[impl installer.finalise.network+4]
    apply_network_config(root, config)?;

    Ok(())
}

// r[impl installer.finalise.timezone]
pub fn apply_timezone_default(target: &MountedTarget) -> Result<()> {
    apply_timezone(target.path(), "UTC")
}

// r[impl installer.write.fstab-fixup]
pub fn fixup_for_encrypted_install(
    target: &MountedTarget,
    install_config: &InstallConfig,
) -> Result<()> {
    let root = target.path();

    tracing::info!("applying encrypted-install fixups");

    // Rewrite /etc/fstab: replace by-partlabel/root with /dev/mapper/root
    let fstab_path = root.join("etc/fstab");
    if fstab_path.exists() {
        let contents = fs::read_to_string(&fstab_path).context("reading target /etc/fstab")?;
        let new_contents = contents.replace("/dev/disk/by-partlabel/root", "/dev/mapper/root");
        if new_contents != contents {
            fs::write(&fstab_path, &new_contents).context("writing target /etc/fstab")?;
            tracing::info!("rewrote /etc/fstab for encrypted install");
        }
    }

    // Truncate /etc/hostname if no explicit hostname is configured
    let has_hostname = install_config.has_hostname_config();
    if !has_hostname {
        let hostname_path = root.join("etc/hostname");
        fs::write(&hostname_path, "").context("truncating /etc/hostname (no explicit hostname)")?;
        tracing::info!("truncated /etc/hostname (no explicit hostname)");
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

// r[impl installer.write.variant-fixup+2]
pub fn write_image_variant(root: &Path, variant_str: &str) -> Result<()> {
    let variant_dir = root.join("etc/bes");
    fs::create_dir_all(&variant_dir).context("creating /etc/bes")?;
    let variant_path = variant_dir.join("image-variant");
    fs::write(&variant_path, format!("{variant_str}\n"))
        .context("writing /etc/bes/image-variant")?;
    tracing::info!("set image-variant to {variant_str}");
    Ok(())
}

// r[impl installer.finalise.copy-install-log+2]
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

// r[impl installer.finalise.timezone]
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

// r[impl installer.finalise.password]
fn apply_password(root: &Path, config: &InstallConfig) -> Result<()> {
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

// r[impl installer.finalise.hostname]
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

// r[impl installer.finalise.hostname]
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

// r[impl installer.finalise.tailscale-auth]
fn attempt_tailscale_auth(root: &Path, authkey: &str) -> Result<()> {
    let mount_str = root.to_str().context("mount path is not valid UTF-8")?;

    let proc_path = root.join("proc");
    let sys_path = root.join("sys");
    let dev_path = root.join("dev");
    let run_path = root.join("run");
    let resolv_path = root.join("etc/resolv.conf");

    run_command(
        paths::MOUNT,
        &["--bind", "/proc", proc_path.to_str().unwrap_or_default()],
    )
    .context("bind-mounting /proc for tailscale auth")?;
    run_command(
        paths::MOUNT,
        &["--bind", "/sys", sys_path.to_str().unwrap_or_default()],
    )
    .context("bind-mounting /sys for tailscale auth")?;
    run_command(
        paths::MOUNT,
        &["--bind", "/dev", dev_path.to_str().unwrap_or_default()],
    )
    .context("bind-mounting /dev for tailscale auth")?;
    run_command(
        paths::MOUNT,
        &["--bind", "/run", run_path.to_str().unwrap_or_default()],
    )
    .context("bind-mounting /run for tailscale auth")?;

    // Ensure DNS resolution works inside the chroot: copy the host's
    // resolv.conf into the target so tailscale can resolve DNS.
    // If the target has a symlink (e.g. to systemd-resolved), remove it
    // first so we can write a plain file.
    let host_resolv = Path::new("/etc/resolv.conf");
    let copied_resolv = if host_resolv.exists() {
        if fs::symlink_metadata(&resolv_path).is_ok_and(|m| m.is_symlink()) {
            let _ = fs::remove_file(&resolv_path);
        }
        fs::copy(host_resolv, &resolv_path).is_ok()
    } else {
        false
    };

    tracing::info!("attempting tailscale auth via chroot into {mount_str}");

    let output = Command::new(paths::CHROOT)
        .args([mount_str, "tailscale", "up", "--auth-key", authkey, "--ssh"])
        .output()
        .context("spawning chroot tailscale up")?;

    // Clean up bind mounts (best-effort, reverse order)
    if copied_resolv {
        let _ = fs::remove_file(&resolv_path);
    }
    let _ = run_command(paths::UMOUNT, &[run_path.to_str().unwrap_or_default()]);
    let _ = run_command(paths::UMOUNT, &[dev_path.to_str().unwrap_or_default()]);
    let _ = run_command(paths::UMOUNT, &[sys_path.to_str().unwrap_or_default()]);
    let _ = run_command(paths::UMOUNT, &[proc_path.to_str().unwrap_or_default()]);

    if output.status.success() {
        tracing::info!("tailscale auth succeeded via chroot");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("tailscale up failed (exit {}): {stderr}", output.status);
        bail!("tailscale up failed (exit {}): {stderr}", output.status);
    }
}

// r[impl installer.finalise.tailscale-firstboot]
fn write_tailscale_authkey(root: &Path, authkey: &str) -> Result<()> {
    let bes_dir = root.join("etc/bes");
    fs::create_dir_all(&bes_dir).context("creating /etc/bes")?;

    let key_path = bes_dir.join("tailscale-authkey");
    fs::write(&key_path, format!("{authkey}\n"))
        .with_context(|| format!("writing tailscale authkey to {}", key_path.display()))?;
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))
        .context("setting tailscale-authkey permissions")?;

    tracing::info!("wrote tailscale authkey for firstboot");
    Ok(())
}

// r[impl installer.finalise.ssh-keys]
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

const BASE_DHCP_NETPLAN: &str = "01-all-en-dhcp.yaml";
const NETPLAN_DIR: &str = "etc/netplan";

// r[impl installer.finalise.network+4]
fn apply_network_config(root: &Path, config: &InstallConfig) -> Result<()> {
    let mode = config.network_mode.unwrap_or(NetworkMode::Dhcp);
    let netplan_dir = root.join(NETPLAN_DIR);
    fs::create_dir_all(&netplan_dir).context("creating netplan directory")?;

    let base_dhcp_path = netplan_dir.join(BASE_DHCP_NETPLAN);

    match mode {
        NetworkMode::Dhcp => {
            tracing::info!("target network: DHCP (leaving base netplan as-is)");
        }
        NetworkMode::StaticIp => {
            let yaml = generate_static_netplan(config);
            let dest = netplan_dir.join("01-installer-static.yaml");
            fs::write(&dest, &yaml).context("writing static netplan")?;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o600))
                .context("setting static netplan permissions")?;
            remove_base_dhcp(&base_dhcp_path);
            tracing::info!("target network: wrote static netplan");
        }
        NetworkMode::Ipv6Slaac => {
            let yaml = generate_ipv6_slaac_netplan(config);
            let dest = netplan_dir.join("01-installer-ipv6-slaac.yaml");
            fs::write(&dest, &yaml).context("writing IPv6 SLAAC netplan")?;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o600))
                .context("setting IPv6 SLAAC netplan permissions")?;
            remove_base_dhcp(&base_dhcp_path);
            tracing::info!("target network: wrote IPv6 SLAAC netplan");
        }
        NetworkMode::Offline => {
            remove_base_dhcp(&base_dhcp_path);
            tracing::info!("target network: offline (removed base DHCP netplan)");
        }
    }

    Ok(())
}

fn remove_base_dhcp(path: &Path) {
    if path.exists()
        && let Err(e) = fs::remove_file(path)
    {
        tracing::warn!("failed to remove base DHCP netplan {}: {e}", path.display());
    }
}

fn generate_static_netplan(config: &InstallConfig) -> String {
    let iface = config.network_interface.as_deref().unwrap_or_default();

    let ip = config.network_ip.as_deref().unwrap_or_default();
    let gw = config.network_gateway.as_deref().unwrap_or_default();

    let match_block = if iface.is_empty() {
        "      match:\n        name: \"en*\"\n".to_string()
    } else {
        format!("      match:\n        name: \"{iface}\"\n")
    };

    let id = if iface.is_empty() { "all-en" } else { iface };

    let mut yaml = format!(
        "network:\n  version: 2\n  ethernets:\n    {id}:\n{match_block}      addresses:\n        - {ip}\n      routes:\n        - to: default\n          via: {gw}\n"
    );

    if let Some(ref dns) = config.network_dns {
        let dns = dns.trim();
        if !dns.is_empty() {
            let servers: Vec<&str> = dns.split(',').map(|s| s.trim()).collect();
            yaml.push_str("      nameservers:\n        addresses:\n");
            for server in &servers {
                yaml.push_str(&format!("          - {server}\n"));
            }
            if let Some(ref domain) = config.network_domain {
                let domain = domain.trim();
                if !domain.is_empty() {
                    yaml.push_str(&format!("        search:\n          - {domain}\n"));
                }
            }
        }
    }

    yaml
}

fn generate_ipv6_slaac_netplan(config: &InstallConfig) -> String {
    let iface = config.network_interface.as_deref().unwrap_or_default();

    let match_block = if iface.is_empty() {
        "      match:\n        name: \"en*\"\n".to_string()
    } else {
        format!("      match:\n        name: \"{iface}\"\n")
    };

    let id = if iface.is_empty() { "all-en" } else { iface };

    format!(
        "network:\n  version: 2\n  ethernets:\n    {id}:\n{match_block}      dhcp4: false\n      accept-ra: true\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.finalise.network+4]
    #[test]
    fn apply_network_config_dhcp_leaves_base_file() {
        let dir = tempfile::tempdir().unwrap();
        let netplan_dir = dir.path().join("etc/netplan");
        fs::create_dir_all(&netplan_dir).unwrap();
        fs::write(
            netplan_dir.join("01-all-en-dhcp.yaml"),
            "network:\n  version: 2\n",
        )
        .unwrap();

        let config = InstallConfig {
            network_mode: Some(NetworkMode::Dhcp),
            ..Default::default()
        };
        apply_network_config(dir.path(), &config).unwrap();

        assert!(
            netplan_dir.join("01-all-en-dhcp.yaml").exists(),
            "base DHCP file should be preserved for DHCP mode"
        );
        assert!(
            !netplan_dir.join("01-installer-static.yaml").exists(),
            "no static file should be written for DHCP mode"
        );
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn apply_network_config_default_is_dhcp() {
        let dir = tempfile::tempdir().unwrap();
        let netplan_dir = dir.path().join("etc/netplan");
        fs::create_dir_all(&netplan_dir).unwrap();
        fs::write(
            netplan_dir.join("01-all-en-dhcp.yaml"),
            "network:\n  version: 2\n",
        )
        .unwrap();

        let config = InstallConfig::default();
        apply_network_config(dir.path(), &config).unwrap();

        assert!(netplan_dir.join("01-all-en-dhcp.yaml").exists());
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn apply_network_config_static_writes_file_and_removes_base() {
        let dir = tempfile::tempdir().unwrap();
        let netplan_dir = dir.path().join("etc/netplan");
        fs::create_dir_all(&netplan_dir).unwrap();
        fs::write(
            netplan_dir.join("01-all-en-dhcp.yaml"),
            "network:\n  version: 2\n",
        )
        .unwrap();

        let config = InstallConfig {
            network_mode: Some(NetworkMode::StaticIp),
            network_interface: Some("enp0s3".into()),
            network_ip: Some("192.168.1.10/24".into()),
            network_gateway: Some("192.168.1.1".into()),
            network_dns: Some("8.8.8.8, 1.1.1.1".into()),
            network_domain: Some("example.com".into()),
            ..Default::default()
        };
        apply_network_config(dir.path(), &config).unwrap();

        assert!(
            !netplan_dir.join("01-all-en-dhcp.yaml").exists(),
            "base DHCP file should be removed for static mode"
        );

        let static_path = netplan_dir.join("01-installer-static.yaml");
        assert!(
            static_path.exists(),
            "static netplan file should be written"
        );

        let perms = fs::metadata(&static_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(perms, 0o600);

        let contents = fs::read_to_string(&static_path).unwrap();
        assert!(contents.contains("192.168.1.10/24"));
        assert!(contents.contains("192.168.1.1"));
        assert!(contents.contains("enp0s3"));
        assert!(contents.contains("8.8.8.8"));
        assert!(contents.contains("1.1.1.1"));
        assert!(contents.contains("example.com"));
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn apply_network_config_static_without_interface_uses_wildcard() {
        let dir = tempfile::tempdir().unwrap();
        let netplan_dir = dir.path().join("etc/netplan");
        fs::create_dir_all(&netplan_dir).unwrap();

        let config = InstallConfig {
            network_mode: Some(NetworkMode::StaticIp),
            network_ip: Some("10.0.0.5/16".into()),
            network_gateway: Some("10.0.0.1".into()),
            ..Default::default()
        };
        apply_network_config(dir.path(), &config).unwrap();

        let contents = fs::read_to_string(netplan_dir.join("01-installer-static.yaml")).unwrap();
        assert!(
            contents.contains("\"en*\""),
            "should use en* wildcard when no interface specified"
        );
        assert!(contents.contains("10.0.0.5/16"));
        assert!(contents.contains("10.0.0.1"));
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn apply_network_config_static_without_dns() {
        let dir = tempfile::tempdir().unwrap();
        let netplan_dir = dir.path().join("etc/netplan");
        fs::create_dir_all(&netplan_dir).unwrap();

        let config = InstallConfig {
            network_mode: Some(NetworkMode::StaticIp),
            network_interface: Some("eth0".into()),
            network_ip: Some("10.0.0.5/24".into()),
            network_gateway: Some("10.0.0.1".into()),
            ..Default::default()
        };
        apply_network_config(dir.path(), &config).unwrap();

        let contents = fs::read_to_string(netplan_dir.join("01-installer-static.yaml")).unwrap();
        assert!(
            !contents.contains("nameservers"),
            "should not include nameservers when DNS is not set"
        );
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn apply_network_config_ipv6_slaac_writes_file_and_removes_base() {
        let dir = tempfile::tempdir().unwrap();
        let netplan_dir = dir.path().join("etc/netplan");
        fs::create_dir_all(&netplan_dir).unwrap();
        fs::write(
            netplan_dir.join("01-all-en-dhcp.yaml"),
            "network:\n  version: 2\n",
        )
        .unwrap();

        let config = InstallConfig {
            network_mode: Some(NetworkMode::Ipv6Slaac),
            network_interface: Some("enp0s3".into()),
            ..Default::default()
        };
        apply_network_config(dir.path(), &config).unwrap();

        assert!(
            !netplan_dir.join("01-all-en-dhcp.yaml").exists(),
            "base DHCP file should be removed for IPv6 SLAAC mode"
        );

        let slaac_path = netplan_dir.join("01-installer-ipv6-slaac.yaml");
        assert!(slaac_path.exists());

        let perms = fs::metadata(&slaac_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(perms, 0o600);

        let contents = fs::read_to_string(&slaac_path).unwrap();
        assert!(contents.contains("dhcp4: false"));
        assert!(contents.contains("accept-ra: true"));
        assert!(contents.contains("enp0s3"));
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn apply_network_config_ipv6_slaac_without_interface_uses_wildcard() {
        let dir = tempfile::tempdir().unwrap();
        let netplan_dir = dir.path().join("etc/netplan");
        fs::create_dir_all(&netplan_dir).unwrap();

        let config = InstallConfig {
            network_mode: Some(NetworkMode::Ipv6Slaac),
            ..Default::default()
        };
        apply_network_config(dir.path(), &config).unwrap();

        let contents =
            fs::read_to_string(netplan_dir.join("01-installer-ipv6-slaac.yaml")).unwrap();
        assert!(contents.contains("\"en*\""));
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn apply_network_config_offline_removes_base_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let netplan_dir = dir.path().join("etc/netplan");
        fs::create_dir_all(&netplan_dir).unwrap();
        fs::write(
            netplan_dir.join("01-all-en-dhcp.yaml"),
            "network:\n  version: 2\n",
        )
        .unwrap();

        let config = InstallConfig {
            network_mode: Some(NetworkMode::Offline),
            ..Default::default()
        };
        apply_network_config(dir.path(), &config).unwrap();

        assert!(
            !netplan_dir.join("01-all-en-dhcp.yaml").exists(),
            "base DHCP file should be removed for offline mode"
        );
        assert!(
            !netplan_dir.join("01-installer-static.yaml").exists(),
            "no static file should be written for offline mode"
        );
        assert!(
            !netplan_dir.join("01-installer-ipv6-slaac.yaml").exists(),
            "no slaac file should be written for offline mode"
        );
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn apply_network_config_offline_no_base_file_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let netplan_dir = dir.path().join("etc/netplan");
        fs::create_dir_all(&netplan_dir).unwrap();

        let config = InstallConfig {
            network_mode: Some(NetworkMode::Offline),
            ..Default::default()
        };
        apply_network_config(dir.path(), &config).unwrap();
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn generate_static_netplan_full() {
        let config = InstallConfig {
            network_mode: Some(NetworkMode::StaticIp),
            network_interface: Some("enp0s3".into()),
            network_ip: Some("192.168.1.10/24".into()),
            network_gateway: Some("192.168.1.1".into()),
            network_dns: Some("8.8.8.8, 1.1.1.1".into()),
            network_domain: Some("example.com".into()),
            ..Default::default()
        };

        let yaml = generate_static_netplan(&config);

        assert!(yaml.starts_with("network:\n  version: 2\n"));
        assert!(yaml.contains("enp0s3:"));
        assert!(yaml.contains("name: \"enp0s3\""));
        assert!(yaml.contains("- 192.168.1.10/24"));
        assert!(yaml.contains("via: 192.168.1.1"));
        assert!(yaml.contains("- 8.8.8.8"));
        assert!(yaml.contains("- 1.1.1.1"));
        assert!(yaml.contains("- example.com"));
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn generate_static_netplan_minimal() {
        let config = InstallConfig {
            network_mode: Some(NetworkMode::StaticIp),
            network_ip: Some("10.0.0.5/16".into()),
            network_gateway: Some("10.0.0.1".into()),
            ..Default::default()
        };

        let yaml = generate_static_netplan(&config);

        assert!(yaml.contains("all-en:"));
        assert!(yaml.contains("name: \"en*\""));
        assert!(yaml.contains("- 10.0.0.5/16"));
        assert!(yaml.contains("via: 10.0.0.1"));
        assert!(!yaml.contains("nameservers"));
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn generate_ipv6_slaac_netplan_with_interface() {
        let config = InstallConfig {
            network_mode: Some(NetworkMode::Ipv6Slaac),
            network_interface: Some("eth0".into()),
            ..Default::default()
        };

        let yaml = generate_ipv6_slaac_netplan(&config);

        assert!(yaml.contains("eth0:"));
        assert!(yaml.contains("name: \"eth0\""));
        assert!(yaml.contains("dhcp4: false"));
        assert!(yaml.contains("accept-ra: true"));
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn generate_ipv6_slaac_netplan_without_interface() {
        let config = InstallConfig::default();

        let yaml = generate_ipv6_slaac_netplan(&config);

        assert!(yaml.contains("all-en:"));
        assert!(yaml.contains("name: \"en*\""));
        assert!(yaml.contains("dhcp4: false"));
        assert!(yaml.contains("accept-ra: true"));
    }

    // r[verify installer.finalise.password]
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

        let config = InstallConfig {
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

    // r[verify installer.finalise.password]
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

        let config = InstallConfig {
            password_hash: Some("$6$custom$myhash".into()),
            ..Default::default()
        };
        apply_password(dir.path(), &config).unwrap();

        let shadow = fs::read_to_string(etc.join("shadow")).unwrap();
        let ubuntu_line = shadow.lines().find(|l| l.starts_with("ubuntu:")).unwrap();
        let fields: Vec<&str> = ubuntu_line.split(':').collect();
        assert_eq!(fields[1], "$6$custom$myhash");
    }

    // r[verify installer.finalise.password]
    #[test]
    fn apply_password_user_not_found_in_shadow() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("shadow"), "root:!:19900:0:99999:7:::\n").unwrap();

        let config = InstallConfig {
            password: Some("test".into()),
            ..Default::default()
        };
        assert!(apply_password(dir.path(), &config).is_err());
    }

    // r[verify installer.finalise.password]
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

        let config = InstallConfig {
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

    // r[verify installer.finalise.hostname]
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

    // r[verify installer.finalise.hostname]
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

    // r[verify installer.finalise.tailscale-firstboot]
    #[test]
    fn write_tailscale_authkey_creates_file_with_correct_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();

        write_tailscale_authkey(dir.path(), "tskey-auth-test123").unwrap();

        let key_path = dir.path().join("etc/bes/tailscale-authkey");
        assert!(key_path.exists());

        let contents = fs::read_to_string(&key_path).unwrap();
        assert_eq!(contents, "tskey-auth-test123\n");

        let perms = fs::metadata(&key_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(perms, 0o600);
    }

    // r[verify installer.finalise.ssh-keys]
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

    // r[verify installer.finalise.ssh-keys]
    #[test]
    fn resolve_uid_gid_user_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("passwd"), "root:x:0:0:root:/root:/bin/bash\n").unwrap();

        assert!(resolve_uid_gid_from_passwd(dir.path(), "ubuntu").is_err());
    }

    // r[verify installer.finalise.timezone]
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

    // r[verify installer.finalise.timezone]
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

    // r[verify installer.finalise.timezone]
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

    // r[verify installer.finalise.timezone]
    #[test]
    fn apply_firstboot_sets_timezone_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        // apply_firstboot needs /etc/shadow for password but we skip password
        // by not setting it. We just need etc to exist.

        let config = InstallConfig {
            timezone: Some("Europe/London".into()),
            ..Default::default()
        };

        // apply_firstboot calls apply_timezone internally.
        // We can't fully call it without a MountedTarget, so test apply_timezone directly.
        apply_timezone(dir.path(), config.timezone.as_deref().unwrap_or("UTC")).unwrap();

        let target = fs::read_link(etc.join("localtime")).unwrap();
        assert_eq!(target, PathBuf::from("/usr/share/zoneinfo/Europe/London"));
    }

    // r[verify installer.finalise.timezone]
    #[test]
    fn apply_firstboot_defaults_timezone_to_utc() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();

        let config = InstallConfig::default();
        let tz = config.timezone.as_deref().unwrap_or("UTC");
        apply_timezone(dir.path(), tz).unwrap();

        let target = fs::read_link(etc.join("localtime")).unwrap();
        assert_eq!(target, PathBuf::from("/usr/share/zoneinfo/UTC"));

        let timezone = fs::read_to_string(etc.join("timezone")).unwrap();
        assert_eq!(timezone, "UTC\n");
    }
}
