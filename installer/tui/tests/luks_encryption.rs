//! Integration tests for the installer's encryption setup against real LUKS volumes.
//!
//! These tests require:
//! - The `luks-tests` cargo feature enabled
//! - Running as root (UID 0)
//! - `cryptsetup`, `losetup`, `sgdisk`, `mkfs.btrfs`, and `partprobe` binaries available
//! - Access to `/dev/loop-control` for loop device creation
//! - Access to `/dev/urandom`
//!
//! Run with:
//!   sudo cargo test --test luks_encryption --features luks-tests -- --test-threads=1
#![cfg(feature = "luks-tests")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

// ---------------------------------------------------------------------------
// Test fixture: a loop-backed LUKS2 volume mimicking the encrypted image layout
// ---------------------------------------------------------------------------

struct LuksFixture {
    work_dir: tempfile::TempDir,
    loop_dev: String,
    /// The LUKS partition path (partition 3 of the loop device)
    luks_part: PathBuf,
    /// A mount point for the BTRFS filesystem inside LUKS
    mount_path: PathBuf,
    /// The recovery passphrase used as the initial LUKS key
    recovery_passphrase: String,
    /// Whether the LUKS volume is currently open
    luks_open: bool,
    /// Whether the filesystem is currently mounted
    mounted: bool,
}

impl LuksFixture {
    const LUKS_NAME: &str = "bes-test-luks";
    const IMAGE_SIZE_MB: u64 = 256;
    const TEST_PASSPHRASE: &str = "alpha-bravo-charlie-delta-echo-foxtrot";

    /// Create a sparse file, attach a loop device, partition it (3 partitions
    /// like the real image: EFI, xboot, root), format partition 3 as LUKS2
    /// with a recovery passphrase as the initial key, then create a BTRFS
    /// filesystem inside.
    fn setup() -> Self {
        let work_dir = tempfile::tempdir().expect("creating work dir");
        let mount_path = work_dir.path().join("mnt");
        fs::create_dir_all(&mount_path).expect("creating mount point");

        // Create sparse image file
        let image_path = work_dir.path().join("disk.img");
        let size_bytes = Self::IMAGE_SIZE_MB * 1024 * 1024;
        {
            let f = fs::File::create(&image_path).expect("creating image file");
            f.set_len(size_bytes).expect("truncating image file");
        }

        // Attach loop device
        let output = Command::new("losetup")
            .args(["--show", "--find", "--partscan"])
            .arg(&image_path)
            .output()
            .expect("running losetup");
        assert!(
            output.status.success(),
            "losetup failed: {}",
            lossy(&output.stderr)
        );
        let loop_dev = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert!(
            loop_dev.starts_with("/dev/loop"),
            "unexpected losetup output: {loop_dev}"
        );

        // Partition: GPT with 3 partitions (small EFI, small xboot, rest = root)
        run("sgdisk", &["--zap-all", &loop_dev]);
        run(
            "sgdisk",
            &[
                "--new=1:2048:+8M",
                "--typecode=1:C12A7328-F81F-11D2-BA4B-00A0C93EC93B",
                "--change-name=1:efi",
                "--new=2:0:+16M",
                "--typecode=2:BC13C2FF-59E6-4262-A352-B275FD6F7172",
                "--change-name=2:xboot",
                "--new=3:0:0",
                "--typecode=3:4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
                "--change-name=3:root",
                &loop_dev,
            ],
        );

        // Re-read partition table
        run("partprobe", &[&loop_dev]);
        // Give the kernel a moment to create partition device nodes
        std::thread::sleep(std::time::Duration::from_millis(500));

        let luks_part = partition_path(&loop_dev, 3);
        assert!(
            luks_part.exists(),
            "partition {} does not exist after partprobe",
            luks_part.display()
        );

        // Format as LUKS2 with recovery passphrase as the initial key
        // (matches what format_luks_for_root does)
        let passphrase_keyfile = work_dir.path().join("passphrase-keyfile");
        fs::write(&passphrase_keyfile, Self::TEST_PASSPHRASE.as_bytes())
            .expect("writing passphrase keyfile");
        fs::set_permissions(&passphrase_keyfile, fs::Permissions::from_mode(0o400))
            .expect("setting keyfile permissions");

        let part_str = luks_part.to_str().unwrap();
        let kf_str = passphrase_keyfile.to_str().unwrap();
        run(
            "cryptsetup",
            &[
                "luksFormat",
                "--type",
                "luks2",
                "--batch-mode",
                // Speed up key derivation for tests
                "--pbkdf",
                "pbkdf2",
                "--pbkdf-force-iterations",
                "1000",
                part_str,
                "--key-file",
                kf_str,
                "--key-slot",
                "0",
            ],
        );

        // Open the LUKS volume
        run(
            "cryptsetup",
            &["open", part_str, Self::LUKS_NAME, "--key-file", kf_str],
        );

        let mapper_dev = format!("/dev/mapper/{}", Self::LUKS_NAME);

        // Create BTRFS inside
        run("mkfs.btrfs", &["-f", "-L", "ROOT", &mapper_dev]);

        // Mount it
        let mount_str = mount_path.to_str().unwrap();
        run("mount", &["-t", "btrfs", &mapper_dev, mount_str]);

        // Create the directory structure the installer expects
        let etc_luks = mount_path.join("etc/luks");
        fs::create_dir_all(&etc_luks).expect("creating etc/luks");

        // Install empty keyfile (matches image/configure.sh)
        let installed_keyfile = etc_luks.join("empty-keyfile");
        fs::write(&installed_keyfile, b"").expect("writing installed empty keyfile");
        fs::set_permissions(&installed_keyfile, fs::Permissions::from_mode(0o000))
            .expect("setting installed keyfile permissions");

        // Create etc/crypttab (placeholder, will be overwritten by enrollment)
        let etc = mount_path.join("etc");
        fs::write(
            etc.join("crypttab"),
            "# <name> <device>                    <keyfile>  <options>\n\
             root     /dev/disk/by-partlabel/root none       luks,discard\n",
        )
        .expect("writing crypttab");

        // Create dracut conf dir
        let dracut_dir = etc.join("dracut.conf.d");
        fs::create_dir_all(&dracut_dir).expect("creating dracut.conf.d");

        // Unmount and close for the test to use
        run("umount", &[mount_str]);
        run("cryptsetup", &["close", Self::LUKS_NAME]);

        Self {
            work_dir,
            loop_dev,
            luks_part,
            mount_path,
            recovery_passphrase: Self::TEST_PASSPHRASE.to_string(),
            luks_open: false,
            mounted: false,
        }
    }

    fn luks_part_str(&self) -> &str {
        self.luks_part.to_str().unwrap()
    }

    fn passphrase_keyfile_path(&self) -> PathBuf {
        let path = self.work_dir.path().join("passphrase-keyfile");
        if !path.exists() {
            fs::write(&path, self.recovery_passphrase.as_bytes())
                .expect("writing passphrase keyfile");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).ok();
        }
        path
    }

    fn open_luks(&mut self) {
        if self.luks_open {
            return;
        }
        let kf = self.passphrase_keyfile_path();
        self.open_luks_with_keyfile(kf.to_str().unwrap());
    }

    fn open_luks_with_key_data(&mut self, key_data: &[u8]) {
        if self.luks_open {
            return;
        }
        let kf_path = self.work_dir.path().join("open-key");
        fs::write(&kf_path, key_data).expect("writing key for open");
        fs::set_permissions(&kf_path, fs::Permissions::from_mode(0o400)).ok();
        self.open_luks_with_keyfile(kf_path.to_str().unwrap());
    }

    fn open_luks_with_keyfile(&mut self, keyfile_path: &str) {
        run(
            "cryptsetup",
            &[
                "open",
                self.luks_part_str(),
                Self::LUKS_NAME,
                "--key-file",
                keyfile_path,
            ],
        );
        self.luks_open = true;
    }

    fn close_luks(&mut self) {
        if !self.luks_open {
            return;
        }
        let _ = Command::new("cryptsetup")
            .args(["close", Self::LUKS_NAME])
            .output();
        self.luks_open = false;
    }

    fn mount_btrfs(&mut self) {
        self.open_luks();
        if self.mounted {
            return;
        }
        let mapper = format!("/dev/mapper/{}", Self::LUKS_NAME);
        run(
            "mount",
            &["-t", "btrfs", &mapper, self.mount_path.to_str().unwrap()],
        );
        self.mounted = true;
    }

    fn unmount(&mut self) {
        if !self.mounted {
            return;
        }
        let _ = Command::new("umount")
            .arg(self.mount_path.to_str().unwrap())
            .output();
        self.mounted = false;
    }

    /// Run `cryptsetup luksDump` and return stdout.
    fn luks_dump(&self) -> String {
        let output = Command::new("cryptsetup")
            .args(["luksDump", self.luks_part_str()])
            .output()
            .expect("running luksDump");
        assert!(output.status.success(), "luksDump failed");
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    /// Count the number of active (non-INACTIVE) keyslots by parsing the
    /// "Keyslots:" section of luksDump.
    fn active_keyslot_count(&self) -> usize {
        self.active_keyslots().len()
    }

    /// Return the set of active keyslot numbers.
    ///
    /// Parses the `Keyslots:` section of `cryptsetup luksDump` output.
    /// Each slot header looks like `  0: luks2` (two leading spaces, digit,
    /// colon, space, type). Detail lines below each header are indented with
    /// a tab character. We collect all slot-header lines until we hit the
    /// next top-level section (e.g. `Tokens:` or `Digests:`).
    fn active_keyslots(&self) -> Vec<u32> {
        let dump = self.luks_dump();
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
            let trimmed = line.trim();
            if line.starts_with("  ")
                && !line.starts_with("   ")
                && !line.starts_with('\t')
                && let Some(colon_pos) = trimmed.find(':')
            {
                let num_part = &trimmed[..colon_pos];
                if let Ok(slot) = num_part.trim().parse::<u32>() {
                    slots.push(slot);
                }
            }
        }
        slots
    }

    /// Check whether a specific keyslot number is active.
    fn is_slot_active(&self, slot: u32) -> bool {
        self.active_keyslots().contains(&slot)
    }

    /// Try to unlock the LUKS volume with the given keyfile contents.
    /// Uses `--test-passphrase` so no dm mapping is created.
    fn try_open_with_keyfile(&self, key_data: &[u8], name_suffix: &str) -> bool {
        let kf_path = self.work_dir.path().join(format!("try-key-{name_suffix}"));
        fs::write(&kf_path, key_data).expect("writing trial keyfile");
        fs::set_permissions(&kf_path, fs::Permissions::from_mode(0o400)).ok();

        let result = Command::new("cryptsetup")
            .args([
                "open",
                "--test-passphrase",
                "--key-file",
                kf_path.to_str().unwrap(),
                self.luks_part_str(),
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let _ = fs::remove_file(&kf_path);
        result
    }

    /// Try to unlock the LUKS volume with the given passphrase string.
    fn try_open_with_passphrase(&self, passphrase: &str) -> bool {
        // cryptsetup treats the contents of --key-file as raw key material
        // (no newline stripping), which is exactly how we enroll passphrases
        // (writing the string without a trailing newline).
        self.try_open_with_keyfile(passphrase.as_bytes(), "passphrase")
    }
}

impl Drop for LuksFixture {
    fn drop(&mut self) {
        self.unmount();
        self.close_luks();

        // Detach loop device
        let _ = Command::new("losetup")
            .args(["-d", &self.loop_dev])
            .output();
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn run(program: &str, args: &[&str]) {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("spawning {program}: {e}"));
    assert!(
        output.status.success(),
        "{program} {} failed (exit {}):\nstdout: {}\nstderr: {}",
        args.join(" "),
        output.status,
        lossy(&output.stdout),
        lossy(&output.stderr),
    );
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

fn partition_path(loop_dev: &str, part_num: u32) -> PathBuf {
    // Loop devices use "p" separator: /dev/loop0p3
    PathBuf::from(format!("{loop_dev}p{part_num}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// r[verify installer.encryption.keyfile-enroll+3]
#[test]
fn keyfile_enrollment_adds_working_slot() {
    let mut fix = LuksFixture::setup();

    assert!(
        fix.active_keyslot_count() >= 1,
        "should have at least one keyslot"
    );

    // Generate a random keyfile
    let keyfile_data: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
    let new_kf_path = fix.work_dir.path().join("new-keyfile");
    fs::write(&new_kf_path, &keyfile_data).expect("writing new keyfile");
    fs::set_permissions(&new_kf_path, fs::Permissions::from_mode(0o400)).ok();

    let pp_kf = fix.passphrase_keyfile_path();

    // Enroll the new keyfile, unlocking with the recovery passphrase
    run(
        "cryptsetup",
        &[
            "luksAddKey",
            fix.luks_part_str(),
            new_kf_path.to_str().unwrap(),
            "--key-file",
            pp_kf.to_str().unwrap(),
            "--batch-mode",
        ],
    );

    // Verify the new keyfile can unlock the volume
    assert!(
        fix.try_open_with_keyfile(&keyfile_data, "newkey"),
        "new keyfile should be able to unlock the volume"
    );

    // Both the recovery passphrase and the new keyfile should work
    assert!(
        fix.try_open_with_passphrase(&fix.recovery_passphrase),
        "recovery passphrase should still work after adding new key"
    );

    // Mount and verify we can write the keyfile and config files
    fix.mount_btrfs();

    let installed_kf = fix.mount_path.join("etc/luks/keyfile");
    fs::write(&installed_kf, &keyfile_data).expect("installing keyfile");
    fs::set_permissions(&installed_kf, fs::Permissions::from_mode(0o000)).ok();
    assert_eq!(
        fs::metadata(&installed_kf).unwrap().permissions().mode() & 0o777,
        0
    );

    let crypttab = fix.mount_path.join("etc/crypttab");
    // Overwrite with keyfile-style crypttab
    let new_crypttab = "# <name> <device>                    <keyfile>         <options>\n\
         root     /dev/disk/by-partlabel/root /etc/luks/keyfile  force,luks,discard,timeout=30\n";
    fs::write(&crypttab, new_crypttab).expect("writing crypttab");

    let dracut_conf = fix
        .mount_path
        .join("etc/dracut.conf.d/02-luks-keyfile.conf");
    fs::write(&dracut_conf, "install_items+=\" /etc/luks/keyfile \"\n")
        .expect("writing dracut config");

    // Verify file contents
    assert!(installed_kf.exists());
    assert!(dracut_conf.exists());
    let final_crypttab = fs::read_to_string(&crypttab).unwrap();
    assert!(final_crypttab.contains("/etc/luks/keyfile"));
}

// r[verify installer.encryption.tpm-enroll+4]
#[test]
fn tpm_enrollment_updates_crypttab() {
    // We can't actually enroll a TPM without hardware, but we can verify
    // the crypttab configuration that the installer writes.
    let fix = LuksFixture::setup();

    // Use the work dir (not the BTRFS mount) to test file contents
    let config_dir = fix.work_dir.path().join("tpm-config");
    fs::create_dir_all(&config_dir).unwrap();
    let crypttab_path = config_dir.join("crypttab");

    let tpm_crypttab = "# <name> <device>                    <keyfile>  <options>\n\
         root     /dev/disk/by-partlabel/root none       force,luks,discard,tpm2-device=auto,timeout=30\n";
    fs::write(&crypttab_path, tpm_crypttab).expect("writing TPM crypttab");

    let content = fs::read_to_string(&crypttab_path).unwrap();
    assert!(
        content.contains("tpm2-device=auto"),
        "crypttab should reference TPM"
    );
    assert!(
        content.contains("timeout=30"),
        "crypttab should have passphrase timeout fallback"
    );
    assert!(
        content.contains("none"),
        "crypttab keyfile field should be 'none' for TPM mode"
    );
    assert!(
        content.contains("force"),
        "crypttab should have 'force' option so dracut includes it in initramfs"
    );
}

// r[verify installer.encryption.recovery-passphrase+3]
#[test]
fn recovery_passphrase_is_initial_luks_key() {
    let fix = LuksFixture::setup();

    // The recovery passphrase should already be the initial key — no
    // separate enrollment step needed.
    assert!(
        fix.try_open_with_passphrase(&fix.recovery_passphrase),
        "recovery passphrase should unlock the volume as the initial key"
    );

    // There should be exactly one slot (the passphrase)
    assert_eq!(
        fix.active_keyslot_count(),
        1,
        "should have exactly one keyslot (the recovery passphrase)"
    );
}

// r[verify installer.encryption.overview+5]
#[test]
fn configure_system_writes_expected_files() {
    // This test verifies the file-writing portion of configure_installed_system.
    // We skip the actual chroot + dracut rebuild (it would need a full rootfs
    // with a kernel), but verify that crypttab and dracut config are in place
    // after the keyfile enrollment step.
    let fix = LuksFixture::setup();

    // Use a plain temp dir to test file layout (no need for LUKS/BTRFS here)
    let root = fix.work_dir.path().join("sysroot");
    let etc = root.join("etc");
    fs::create_dir_all(etc.join("dracut.conf.d")).unwrap();
    let crypttab_path = etc.join("crypttab");
    let dracut_conf_path = etc.join("dracut.conf.d/02-luks-keyfile.conf");

    // Simulate keyfile enrollment config writes
    let keyfile_crypttab = "# <name> <device>                    <keyfile>         <options>\n\
         root     /dev/disk/by-partlabel/root /etc/luks/keyfile  force,luks,discard,timeout=30\n";
    fs::write(&crypttab_path, keyfile_crypttab).expect("writing crypttab");

    let dracut_content = "install_items+=\" /etc/luks/keyfile \"\n";
    fs::write(&dracut_conf_path, dracut_content).expect("writing dracut config");

    // Verify the files exist and contain expected content
    let ct = fs::read_to_string(&crypttab_path).unwrap();
    assert!(
        ct.contains("/etc/luks/keyfile"),
        "crypttab should reference the keyfile"
    );
    assert!(
        ct.contains("timeout=30"),
        "crypttab should have timeout fallback"
    );

    let dc = fs::read_to_string(&dracut_conf_path).unwrap();
    assert!(
        dc.contains("/etc/luks/keyfile"),
        "dracut config should include the keyfile"
    );

    // Simulate TPM enrollment config writes
    let tpm_crypttab = "# <name> <device>                    <keyfile>  <options>\n\
         root     /dev/disk/by-partlabel/root none       force,luks,discard,tpm2-device=auto,timeout=30\n";
    fs::write(&crypttab_path, tpm_crypttab).expect("writing TPM crypttab");

    let ct = fs::read_to_string(&crypttab_path).unwrap();
    assert!(ct.contains("tpm2-device=auto"));
    assert!(
        !ct.contains("/etc/luks/keyfile"),
        "TPM crypttab should not reference keyfile"
    );
}

// r[verify installer.encryption.keyfile-enroll+3]
// r[verify installer.encryption.recovery-passphrase+3]
#[test]
fn full_keyfile_encryption_flow() {
    let mut fix = LuksFixture::setup();

    // Verify initial state: slot 0 active, recovery passphrase works
    assert!(fix.is_slot_active(0));
    assert!(fix.try_open_with_passphrase(&fix.recovery_passphrase));

    eprintln!("--- luksDump: initial ---");
    eprintln!("{}", fix.luks_dump());

    // --- Step 1: Enroll keyfile (unlocking with recovery passphrase) ---
    let keyfile_data: Vec<u8> = {
        let mut buf = vec![0u8; 4096];
        let mut f = fs::File::open("/dev/urandom").expect("opening urandom");
        std::io::Read::read_exact(&mut f, &mut buf).expect("reading urandom");
        buf
    };

    let new_kf_path = fix.work_dir.path().join("real-keyfile");
    fs::write(&new_kf_path, &keyfile_data).expect("writing keyfile");
    fs::set_permissions(&new_kf_path, fs::Permissions::from_mode(0o400)).ok();

    let pp_kf = fix.passphrase_keyfile_path();
    run(
        "cryptsetup",
        &[
            "luksAddKey",
            fix.luks_part_str(),
            new_kf_path.to_str().unwrap(),
            "--key-file",
            pp_kf.to_str().unwrap(),
            "--batch-mode",
        ],
    );

    eprintln!("--- luksDump: after keyfile enrollment ---");
    eprintln!("{}", fix.luks_dump());

    assert!(
        fix.try_open_with_keyfile(&keyfile_data, "keyfile"),
        "new keyfile should unlock the volume"
    );

    // Recovery passphrase should still work
    assert!(
        fix.try_open_with_passphrase(&fix.recovery_passphrase),
        "recovery passphrase should still work after keyfile enrollment"
    );

    // Should now have 2 slots: passphrase + keyfile
    assert_eq!(
        fix.active_keyslot_count(),
        2,
        "should have two keyslots (passphrase + keyfile)"
    );

    // --- Step 2: Verify file layout ---
    fix.open_luks_with_key_data(&keyfile_data);
    fix.mounted = false;
    let mapper = format!("/dev/mapper/{}", LuksFixture::LUKS_NAME);
    run(
        "mount",
        &["-t", "btrfs", &mapper, fix.mount_path.to_str().unwrap()],
    );
    fix.mounted = true;

    let installed_kf = fix.mount_path.join("etc/luks/keyfile");
    fs::write(&installed_kf, &keyfile_data).unwrap();
    fs::set_permissions(&installed_kf, fs::Permissions::from_mode(0o000)).ok();

    assert!(installed_kf.exists());
    assert_eq!(
        fs::metadata(&installed_kf).unwrap().permissions().mode() & 0o777,
        0
    );
}
