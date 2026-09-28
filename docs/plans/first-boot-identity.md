# First-boot identity rotation

Images flashed or booted directly (not through the installer) share every
identifier baked in at build time: the LUKS master key, the LUKS UUID, the
btrfs filesystem ID, the xboot ext4 UUID, the FAT serial, and the GPT disk
GUID and PARTUUIDs. The installer already produces fresh values for all of
these; nothing does for a flashed pi, a directly-booted metal qcow2/vmdk/raw,
or a cloud image. The shared LUKS master key is the security issue: rotating
the empty passphrase leaves it unchanged, so anyone with the published image
can decrypt any device.

The metal-only `luks-reencrypt.service` removed in `ac60b6a` (2026-03-05)
rotated the master key online after growth; this replaces it with a broader,
cheaper mechanism for every variant.

## Decisions

- Scope: `metal`, `pi`, `cloud`. Installer-built systems are left alone by
  construction (their identifiers no longer match the build record).
- Everything runs in the initramfs, before the root filesystem is mounted and
  before `grow-root-filesystem`. Re-encryption is offline and only covers the
  image-sized area. First boot is held for its duration.
- Trigger: each identifier is rotated only while it still equals its value in
  a build-time record. No marker files.
- Re-encryption is guarded on the LUKS volume-key digest, not the LUKS UUID:
  it runs while the digest still equals the recorded one, and resumes if the
  header shows a re-encryption in progress. The LUKS UUID is then an ordinary
  identifier, rotated after re-encryption completes. A crash between
  re-encryption and the UUID change therefore never re-encrypts twice.
- GRUB variants (`metal`, `cloud`) keep the xboot ext4 UUID at its build
  value: the GRUB core image on the ESP embeds it, and it cannot be rewritten
  from the initramfs without an unrecoverable power-loss window. `pi`
  rotates it.
- GRUB variants rewrite `grub.cfg` (text, on xboot) whenever it references a
  build value, atomically, on every boot. If the running cmdline referenced
  an identifier that was just rotated, the system reboots once repaired. The
  normal first boot and the interrupted-repair case take the same path.
- btrfs: `btrfstune -u` (parity with the installer, no metadata_uuid
  incompat flag).

## Global constraints

- Follow AGENTS.md: spec text says what, not how (no tool names or command
  lines in spec prose); tracey annotations on implementation and tests
  (`r[...]` bare in image scripts, `r[verify ...]` in tests, matching the
  surrounding files); shell must pass `shellcheck`; use jj, commit granularly.
- Every rotation step is guarded by "current value == recorded value" and is
  safe to re-run after interruption at any point.
- GRUB variants never change the xboot ext4 UUID.
- Nothing in the image may reference GPT GUIDs, the LUKS UUID, the btrfs
  ID, or the FAT serial except `grub.cfg` (repaired) — fstab/crypttab use
  partition labels and `/dev/mapper/root`.

## Task 1: Spec

File: `docs/spec/disk-images.md`.


New section "First-boot identity", prose acceptance criteria:

- `image.identity.record`: the image carries a record of its build-time
  identifiers (GPT disk GUID, each PARTUUID, LUKS UUID and volume-key digest
  where encrypted, btrfs filesystem ID, xboot UUID, boot1 FAT serial), present
  in both the root filesystem and the initramfs.
- `image.identity.rotate`: at boot, before the root filesystem is mounted,
  every identifier still equal to its recorded value is replaced with a
  fresh random one; the xboot UUID is exempt on GRUB variants (with the
  reason). Systems whose identifiers already differ are untouched.
- `image.identity.luks-rekey`: on encrypted variants, while the volume's
  master key is still the one it was built with (or a re-encryption is in
  progress) the volume is re-encrypted under a fresh master key before it is
  unlocked for use; interruption at any point must leave a volume that the
  next boot unlocks and finishes re-encrypting; this completes before root
  growth, so only the image-sized area is rewritten.
- `image.identity.grub-repair`: on GRUB variants, a `grub.cfg` that
  references a recorded value is rewritten to the current values without a
  window in which it is partially written; if the running boot was started
  from a reference that no longer resolves, the system reboots once the
  repair is durable.

Bump `r[image.variant.types+3]` → `+4`: directly-booted encrypted images
rotate their own master key on first boot; the installer still does so for
installed systems.


## Task 2: Build-time record


`image/build.sh`: after filesystems are created and before the chroot
runs, write the record to `$MNT/etc/bes/build-identity`, mode 0644, as
shell-sourceable `KEY=value` lines (values lowercase where hex):

- `DISK_GUID`: GPT disk GUID
- `PARTUUID_1`, `PARTUUID_2`, `PARTUUID_3`: GPT partition GUIDs
- `BOOT1_SERIAL`: FAT volume serial as `blkid` reports it (`XXXX-XXXX`)
- `XBOOT_UUID`: ext4 UUID of partition 2
- `BTRFS_UUID`: btrfs filesystem ID
- `LUKS_UUID`: LUKS2 header UUID (metal, pi only)
- `LUKS_DIGEST`: the volume-key digest value of digest 0 from the LUKS2
  header (metal, pi only)

Keys that do not apply to the variant are omitted. Partitions are never
changed after this point in the build (grow happens on the device), so the
values stay valid.

Add to `tests/test-image-structure.sh`: the record exists in the rootfs and
each recorded value matches the image's actual identifier (read from the
loop device / LUKS header / filesystems the test already opens).

## Task 3: Initramfs rotation module

2. New dracut module `image/files/dracut/modules.d/90bes-identity/`:
   - `module-setup.sh`: installs the record, the script, the units, and the
     tools it needs (`sgdisk`, `partx`, `cryptsetup`, `btrfstune`, `tune2fs`,
     `e2fsck`, `mlabel`, `blkid`); `depends` on `systemd` and `crypt` where
     relevant.
   - `bes-identity-pre.service`: after the `root` partition-label device
     appears, `Before=cryptsetup-pre.target` (so `systemd-cryptsetup@root`
     waits); rotates GPT GUIDs and re-reads the table; on encrypted
     variants resumes an in-progress re-encryption or re-encrypts if the
     volume-key digest is still `LUKS_DIGEST` (unlocking with the empty
     keyfile already in the initramfs), then rotates the LUKS UUID.
     `TimeoutStartSec=infinity`, progress on the console.
   - `bes-identity-post.service`: after `cryptsetup.target` and the btrfs
     device (`/dev/mapper/root` on encrypted variants, the `root`
     partition on cloud), `Before=sysroot.mount` and before any fsck of the
     affected filesystems; rotates btrfs ID, FAT serial, xboot UUID (pi
     only); then on GRUB variants mounts xboot, repairs `grub.cfg`, and if
     `/proc/cmdline` references a rotated value, syncs and reboots.
   - The kernel cmdline on GRUB variants is `root=UUID=<btrfs id>`, so the
     reboot is the normal first-boot path there, not an edge case. The pi
     cmdline uses `/dev/mapper/root` and never needs it.
   - The live installer ISO builds its own rootfs and never carries this
     module; no ISO change.
   - `bes-identity`: one script with `pre`/`post` subcommands; every step
     guarded by "current value == recorded value".
3. `image/configure.sh`: install the module and add it via
   `image/files/dracut/05-bes-identity.conf` (`add_dracutmodules+=`), before
   the dracut run.
4. Ensure `mtools` is installed in all variants (source of `mlabel`) if not
   already pulled in.
5. Installer: confirm the module is a no-op on installed systems (all
   identifiers differ). No installer change expected; if the installer's
   hostonly rebuild drops the module, that is fine too.


Add to `tests/test-image-structure.sh`: the record and the module's units
are present in the initramfs: every `/boot/initrd.img-*`, and on pi also
`/boot/firmware/current/initrd.img` (the one the firmware boots).

## Task 4: Boot test

`justfile` in-guest cloud-init test script: after boot, each recorded
identifier differs from the current one (xboot excepted on GRUB variants);
on encrypted variants the LUKS volume-key digest differs; `grub.cfg`
references only current UUIDs (existing `image.boot.grub-uuids` check keeps
holding).

`test-boot` runs QEMU with `-no-reboot`, which would end the run at the
GRUB variants' planned repair reboot. Allow exactly the reboots the module
makes (the guest powers off at the end of the script, and the existing
timeout still bounds a reboot loop).
