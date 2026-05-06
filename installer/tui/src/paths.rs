use std::path::Path;
use std::sync::OnceLock;

// Absolute paths for external binaries used by the installer.
//
// Hardcoding these avoids reliance on `PATH` in the live ISO environment,
// where the shell/systemd context may not include `/usr/sbin` or `/sbin`.

// coreutils / util-linux
pub const MOUNT: &str = "/usr/bin/mount";
pub const UMOUNT: &str = "/usr/bin/umount";
pub const MKNOD: &str = "/usr/bin/mknod";

// Ubuntu 26.04 (resolute) split coreutils into alternative providers
// (`coreutils-from-{gnu,uutils,busybox,toybox}`) that ship `chroot` at
// different paths: GNU at /usr/sbin/chroot, the rest at /usr/bin/chroot.
// We don't pin a specific provider in the live rootfs, so resolve the
// path at runtime against whichever is actually installed.
pub const CHROOT_CANDIDATES: &[&str] = &["/usr/bin/chroot", "/usr/sbin/chroot"];

/// First-existing path for the `chroot` binary, or the first candidate as
/// a fallback. Cached on first call.
pub fn chroot() -> &'static str {
    static CACHED: OnceLock<&'static str> = OnceLock::new();
    CACHED.get_or_init(|| {
        for c in CHROOT_CANDIDATES {
            if Path::new(c).exists() {
                return c;
            }
        }
        CHROOT_CANDIDATES[0]
    })
}
pub const LSBLK: &str = "/usr/bin/lsblk";
pub const BLKID: &str = "/usr/sbin/blkid";
pub const SFDISK: &str = "/usr/sbin/sfdisk";
pub const WIPEFS: &str = "/usr/sbin/wipefs";
pub const UDEVADM: &str = "/usr/bin/udevadm";

// gdisk
pub const SGDISK: &str = "/usr/sbin/sgdisk";

// parted
pub const PARTPROBE: &str = "/usr/sbin/partprobe";

// cryptsetup / veritysetup
pub const CRYPTSETUP: &str = "/usr/sbin/cryptsetup";
pub const VERITYSETUP: &str = "/usr/sbin/veritysetup";

// loop devices
pub const LOSETUP: &str = "/usr/sbin/losetup";

// filesystem tools
pub const BTRFS: &str = "/usr/bin/btrfs";
pub const BTRFSTUNE: &str = "/usr/bin/btrfstune";
pub const E2FSCK: &str = "/usr/sbin/e2fsck";
pub const TUNE2FS: &str = "/usr/sbin/tune2fs";
pub const MLABEL: &str = "/usr/bin/mlabel";

// initramfs (used inside chroot into target system)
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
pub const IP: &str = "/usr/sbin/ip";
pub const NETPLAN: &str = "/usr/sbin/netplan";
pub const TAILSCALE: &str = "/usr/bin/tailscale";

// Binaries executed directly by the installer in the live ISO environment.
// `chroot` is checked separately (see CHROOT_CANDIDATES — it can live at
// any of multiple paths depending on which coreutils alternative shipped).
const ISO_PATHS: &[(&str, &str)] = &[
    ("mount", MOUNT),
    ("umount", UMOUNT),
    ("mknod", MKNOD),
    ("lsblk", LSBLK),
    ("blkid", BLKID),
    ("sfdisk", SFDISK),
    ("wipefs", WIPEFS),
    ("udevadm", UDEVADM),
    ("sgdisk", SGDISK),
    ("partprobe", PARTPROBE),
    ("cryptsetup", CRYPTSETUP),
    ("veritysetup", VERITYSETUP),
    ("losetup", LOSETUP),
    ("btrfs", BTRFS),
    ("btrfstune", BTRFSTUNE),
    ("e2fsck", E2FSCK),
    ("tune2fs", TUNE2FS),
    ("mlabel", MLABEL),
    ("systemctl", SYSTEMCTL),
    ("systemd-cryptenroll", SYSTEMD_CRYPTENROLL),
    ("reboot", REBOOT),
    ("chvt", CHVT),
    ("bash", BASH),
    ("curl", CURL),
    ("ip", IP),
    ("netplan", NETPLAN),
    ("tailscale", TAILSCALE),
];

// Binaries invoked inside a chroot into the target system (not needed in the
// live ISO squashfs).
const CHROOT_PATHS: &[(&str, &str)] = &[("dracut", DRACUT)];

fn check_paths(paths: &[(&str, &str)], sysroot: Option<&Path>) -> Result<(), String> {
    let mut missing: Vec<String> = Vec::new();

    for &(name, path) in paths {
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

/// Check that every hardcoded ISO-environment binary path exists on disk.
///
/// Returns `Ok(())` if all paths are present, or `Err` with a message listing
/// every missing binary. When `sysroot` is `Some`, paths are resolved relative
/// to that directory (e.g. a mounted squashfs).
pub fn check_iso(sysroot: Option<&Path>) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();
    if let Err(e) = check_paths(ISO_PATHS, sysroot) {
        errors.push(e);
    }
    if let Err(e) = check_any_path("chroot", CHROOT_CANDIDATES, sysroot) {
        errors.push(e);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn check_any_path(name: &str, candidates: &[&str], sysroot: Option<&Path>) -> Result<(), String> {
    let any = candidates.iter().any(|p| {
        let full = match sysroot {
            Some(root) => root.join(p.strip_prefix('/').unwrap_or(p)),
            None => Path::new(p).to_path_buf(),
        };
        full.exists()
    });
    if any {
        Ok(())
    } else {
        let checked: Vec<String> = candidates
            .iter()
            .map(|p| match sysroot {
                Some(root) => root
                    .join(p.strip_prefix('/').unwrap_or(p))
                    .display()
                    .to_string(),
                None => (*p).to_string(),
            })
            .collect();
        Err(format!(
            "1 missing binary path(s):\n  {name}: any of {} (checked: {})",
            candidates.join(", "),
            checked.join(", "),
        ))
    }
}

/// Check that every hardcoded chroot-target binary path exists on disk.
///
/// These are binaries invoked inside a `chroot` into the installed system
/// (e.g. `dracut`). They do not need to be present in the live ISO rootfs.
pub fn check_chroot(sysroot: Option<&Path>) -> Result<(), String> {
    check_paths(CHROOT_PATHS, sysroot)
}
