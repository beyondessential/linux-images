use std::path::Path;

// Absolute paths for external binaries used by the installer.
//
// Hardcoding these avoids reliance on `PATH` in the live ISO environment,
// where the shell/systemd context may not include `/usr/sbin` or `/sbin`.

// coreutils / util-linux
pub const MOUNT: &str = "/usr/bin/mount";
pub const UMOUNT: &str = "/usr/bin/umount";
pub const MKNOD: &str = "/usr/bin/mknod";
pub const CHROOT: &str = "/usr/sbin/chroot";
pub const LSBLK: &str = "/usr/bin/lsblk";
pub const BLKID: &str = "/usr/sbin/blkid";
pub const SFDISK: &str = "/usr/sbin/sfdisk";
pub const WIPEFS: &str = "/usr/sbin/wipefs";
pub const UDEVADM: &str = "/usr/bin/udevadm";

// gdisk
pub const SGDISK: &str = "/usr/sbin/sgdisk";

// parted
pub const PARTPROBE: &str = "/usr/sbin/partprobe";

// cryptsetup
pub const CRYPTSETUP: &str = "/usr/sbin/cryptsetup";

// filesystem tools
pub const BTRFS: &str = "/usr/bin/btrfs";
pub const BTRFSTUNE: &str = "/usr/bin/btrfstune";
pub const E2FSCK: &str = "/usr/sbin/e2fsck";
pub const TUNE2FS: &str = "/usr/sbin/tune2fs";
pub const MLABEL: &str = "/usr/bin/mlabel";

// initramfs
pub const DRACUT: &str = "/usr/bin/dracut";

// systemd
pub const SYSTEMCTL: &str = "/usr/bin/systemctl";
pub const SYSTEMD_CRYPTENROLL: &str = "/usr/bin/systemd-cryptenroll";
pub const REBOOT: &str = "/sbin/reboot";

// kbd
pub const CHVT: &str = "/usr/bin/chvt";

// shells
pub const BASH: &str = "/bin/bash";

// networking
pub const CURL: &str = "/usr/bin/curl";
pub const TAILSCALE: &str = "/usr/bin/tailscale";

// r[impl installer.check-paths]
const ALL_PATHS: &[(&str, &str)] = &[
    ("mount", MOUNT),
    ("umount", UMOUNT),
    ("mknod", MKNOD),
    ("chroot", CHROOT),
    ("lsblk", LSBLK),
    ("blkid", BLKID),
    ("sfdisk", SFDISK),
    ("wipefs", WIPEFS),
    ("udevadm", UDEVADM),
    ("sgdisk", SGDISK),
    ("partprobe", PARTPROBE),
    ("cryptsetup", CRYPTSETUP),
    ("btrfs", BTRFS),
    ("btrfstune", BTRFSTUNE),
    ("e2fsck", E2FSCK),
    ("tune2fs", TUNE2FS),
    ("mlabel", MLABEL),
    ("dracut", DRACUT),
    ("systemctl", SYSTEMCTL),
    ("systemd-cryptenroll", SYSTEMD_CRYPTENROLL),
    ("reboot", REBOOT),
    ("chvt", CHVT),
    ("bash", BASH),
    ("curl", CURL),
    ("tailscale", TAILSCALE),
];

/// Check that every hardcoded binary path exists on disk.
///
/// Returns `Ok(())` if all paths are present, or `Err` with a message listing
/// every missing binary. When `sysroot` is `Some`, paths are resolved relative
/// to that directory (e.g. a mounted squashfs).
pub fn check_all(sysroot: Option<&Path>) -> Result<(), String> {
    let mut missing: Vec<String> = Vec::new();

    for &(name, path) in ALL_PATHS {
        let full = match sysroot {
            Some(root) => root.join(path.strip_prefix('/').unwrap_or(path)),
            None => Path::new(path).to_path_buf(),
        };
        if !full.exists() {
            missing.push(format!("  {name}: {path} (checked: {})", full.display()));
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} missing binary path(s):\n{}",
            missing.len(),
            missing.join("\n")
        ))
    }
}
