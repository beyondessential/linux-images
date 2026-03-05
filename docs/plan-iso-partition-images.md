# Plan: ISO Size Reduction via Partition-Level Images

## Summary

Reduce the ISO size by roughly half. Instead of embedding both the metal and
cloud `.raw.zst` disk images (which contain nearly identical OS content), the
ISO build extracts the cloud image's three partitions into separate compressed
files. The installer then creates the GPT table itself and writes each
partition individually. When encryption is enabled, the installer sets up LUKS
on the root partition before writing the filesystem content into it.

Both the metal and cloud full-disk images are still built and published as
standalone artifacts for direct-imaging workflows. Only the ISO payload
changes.

## Current state

The disk encryption rework is complete. Specifically:

- `DiskEncryption` is an enum (`Tpm`, `Keyfile`, `None`) in
  `installer/tui/src/config.rs`. The variant is derived from it
  (`Tpm`/`Keyfile` -> `metal`, `None` -> `cloud`).
- The installer already performs all LUKS setup after writing the image:
  key rotation, TPM/keyfile enrollment, recovery passphrase, empty-slot
  wipe, initramfs rebuild (see `installer/tui/src/encryption.rs`,
  `run_encryption_setup`).
- The metal image ships with LUKS2 + empty passphrase; the installer
  rotates the key and enrolls the real mechanism at install time.

The ISO currently embeds both `*-metal-<arch>-*.raw.zst` and
`*-cloud-<arch>-*.raw.zst`. These are complete GPT disk images (EFI + xboot
+ root). The installer picks one based on the variant derived from
`DiskEncryption`, decompresses it as a whole-disk stream to the target
device, then expands partitions to fill the disk.

The metal and cloud images differ only in:

1. The root partition: metal wraps BTRFS in LUKS2; cloud has bare BTRFS.
2. A handful of config files: `/etc/crypttab`, `/etc/fstab`,
   `/etc/bes/image-variant`, `/etc/luks/empty-keyfile`, dracut LUKS config,
   `/etc/hostname`.

This means the ISO ships approximately 2x the compressed data for what is
essentially the same OS content.

### Relevant code paths today

- **`iso/build-iso.sh` Phase 5**: Copies all `*.raw.zst` files (and their
  `.size` sidecars) from `IMAGE_DIR` into `$STAGING/images/`.
- **`justfile` `iso` recipe**: Requires both metal and cloud `.raw.zst`
  images to exist under `output/<arch>/`.
- **`writer::find_image_path`**: Searches `/run/live/medium/images` (and
  fallback dirs) for a file matching `{variant}-{arch}` ending in `.raw.zst`.
- **`writer::write_image`**: Calls `wipe_disk`, then stream-decompresses
  the entire `.raw.zst` to the target block device.
- **`writer::image_uncompressed_size`**: Reads a `.size` sidecar (whole
  disk image size).
- **`writer::expand_partitions`**: Moves GPT secondary header, runs
  `growpart` on partition 3, re-reads table.
- **`writer::verify_partition_table`**: Uses `sfdisk --json` to check for
  `efi`, `xboot`, `root` partition labels.
- **`main.rs` `run_auto`**: Calls `find_image_path` with variant+arch,
  then `write_image`, `reread_partition_table`, `verify_partition_table`,
  `expand_partitions`, firstboot, then encryption setup.
- **`ui/run.rs` `start_write_worker`**: Same flow for interactive mode.
- **`plan.rs` `InstallPlan`**: Has `image_path: Option<PathBuf>` field.
- **`firstboot.rs` `mount_target`**: Opens LUKS with empty keyfile if
  encrypted, then mounts btrfs `subvol=@`.
- **`encryption.rs` `run_encryption_setup`**: Operates on the already-
  written metal image's LUKS partition (rotates key, enrolls mechanism,
  etc.).
- **`tests/test-iso-structure.sh`**: Checks for `*.raw.zst` images on
  ISO, checks for metal and cloud images by name pattern, checks `.size`
  sidecars.

## Design

### ISO contents (new)

The ISO will contain three compressed partition images extracted from the
cloud image, plus metadata:

```
/images/efi.img.zst            # FAT32 EFI System Partition (512 MiB uncompressed)
/images/efi.img.size           # uncompressed byte count
/images/xboot.img.zst          # ext4 extended boot partition (1 GiB uncompressed)
/images/xboot.img.size
/images/root.img.zst           # bare BTRFS root partition (~3.5 GiB uncompressed)
/images/root.img.size
/images/partitions.json        # partition geometry and type UUIDs (see below)
```

The `partitions.json` file records everything the installer needs to
reconstruct the GPT:

```json
{
  "arch": "amd64",
  "partitions": [
    {
      "label": "efi",
      "type_uuid": "C12A7328-F81F-11D2-BA4B-00A0C93EC93B",
      "size_mib": 512,
      "image": "efi.img.zst"
    },
    {
      "label": "xboot",
      "type_uuid": "BC13C2FF-59E6-4262-A352-B275FD6F7172",
      "size_mib": 1024,
      "image": "xboot.img.zst"
    },
    {
      "label": "root",
      "type_uuid": "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
      "size_mib": 0,
      "image": "root.img.zst"
    }
  ]
}
```

`size_mib: 0` means "use all remaining space" (same as `sgdisk -n 3:0:0`).
The root type UUID is architecture-dependent (the value above is for amd64;
arm64 uses `B921B045-1DF0-41C3-AF44-4C6F280D3FAE`).

### Installer write flow (new)

Current flow (`run_auto` / `start_write_worker`):

1. `find_image_path(variant, arch)` -- locates the `.raw.zst` by variant.
2. `image_uncompressed_size` -- reads `.size` sidecar for disk size check.
3. `check_disk_size` -- compares image size to target disk.
4. `write_image` -- calls `wipe_disk`, then streams whole `.raw.zst` to
   block device.
5. `reread_partition_table`.
6. `verify_partition_table`.
7. `expand_partitions` -- moves GPT header, growpart on partition 3.
8. Firstboot (mount, apply config, unmount).
9. Encryption setup (if encrypted).

New flow:

1. `find_partition_manifest()` -- locates and parses `partitions.json`.
2. Compute total uncompressed size from the three `.size` sidecars.
3. `check_disk_size` -- sum of fixed partition sizes + root image size +
   GPT overhead.
4. `wipe_disk`.
5. `create_partition_table` -- create GPT and three partitions via `sgdisk`
   using geometry from `partitions.json`.
6. `reread_partition_table` (includes `ensure_partition_devices`).
7. Stream-decompress `efi.img.zst` -> `/dev/sdX1`.
8. Stream-decompress `xboot.img.zst` -> `/dev/sdX2`.
9. Root partition:
   - If encryption is `None`: stream-decompress `root.img.zst` -> `/dev/sdX3`.
   - If encryption is `Tpm` or `Keyfile`:
     a. `cryptsetup luksFormat --type luks2` on `/dev/sdX3` with empty
        passphrase.
     b. `cryptsetup open` -> `/dev/mapper/root`.
     c. Stream-decompress `root.img.zst` -> `/dev/mapper/root`.
     d. `cryptsetup close root`.
10. Expand root filesystem:
    - If encryption is `Tpm` or `Keyfile`: `cryptsetup open` (if not
      already open), `cryptsetup resize root`, then
      `btrfs filesystem resize max` on the mounted filesystem.
    - If encryption is `None`: `btrfs filesystem resize max` on the
      mounted filesystem.
    This requires a temporary mount of the BTRFS (subvol `@`), resize,
    then unmount. This ensures the installed system has a fully expanded
    filesystem and does not depend on `grow-root-filesystem.service`.
11. Randomize filesystem UUIDs (see below).
12. Rebuild initramfs and GRUB config (unconditionally, see below).
13. `verify_partition_table`.
14. Firstboot (mount, apply config, unmount). When encryption is not
    `None`, also fix up fstab and write variant marker.
15. Encryption setup (if encrypted) -- same as today: `run_encryption_setup`
    handles key rotation, enrollment, recovery passphrase, initramfs.
    Note: `configure_installed_system` in `encryption.rs` already rebuilds
    the initramfs; since step 12 now does this unconditionally, the
    encryption path will rebuild it a second time (picking up crypttab and
    keyfile changes). This is intentional and harmless.

Since the installer creates the GPT with partition 3 spanning all remaining
space, neither `growpart` nor `--move-second-header` is needed. The BTRFS
filesystem inside `root.img.zst` is smaller than the partition it is written
into, so the installer must resize it to fill the available space. This is
done at install time rather than deferring to a boot-time service.

### Config fixups

The cloud image's root filesystem has cloud-style config. When the installer
is producing a metal variant (encryption is not `None`), it must patch
certain files during the firstboot mount phase. This is a new
responsibility since we are now always starting from cloud partition images.

Files to fix up when encryption is not `None`:

- **`/etc/fstab`**: Replace `by-partlabel/root` with `/dev/mapper/root`
  for the root (`/`) and postgresql (`/var/lib/postgresql`) mount entries.
- **`/etc/bes/image-variant`**: Write `metal`. (Today the image already
  has the correct value baked in because the installer picks the right
  image. Now we always start from cloud and must fix it up.)
- **`/etc/hostname`**: The cloud image ships `ubuntu` as the hostname; the
  metal image ships an empty file. When encryption is enabled and no
  explicit hostname is configured, truncate to empty (matching metal
  behavior). When an explicit hostname is set, `apply_hostname` in
  firstboot already handles it.

The following are already handled by `encryption.rs`
`configure_installed_system` and do not need new code:

- `/etc/crypttab` creation
- `/etc/luks/empty-keyfile` creation
- Dracut LUKS keyfile config
- Initramfs rebuild

When encryption is `None`, no fixups are needed -- the cloud image's fstab
and config are already correct. The variant marker `/etc/bes/image-variant`
should be left as `cloud` (or written explicitly to `cloud` for clarity).

### LUKS setup before write

This is a new phase that does not exist today. Currently the metal image
already has LUKS on the root partition, and the installer writes it as-is.
In the new flow, the installer must create the LUKS container on the raw
partition before writing the BTRFS content into it.

The initial LUKS setup uses an empty passphrase (matching what the metal
image build does today via `image.luks.format`). The subsequent
`run_encryption_setup` phase then rotates the key, enrolls the real
mechanism, and wipes the empty slot -- exactly as it does today.

Implementation:

```
cryptsetup luksFormat --type luks2 --key-file /tmp/bes-empty-keyfile /dev/sdX3
cryptsetup open --type luks2 --key-file /tmp/bes-empty-keyfile /dev/sdX3 bes-target-root
# stream-decompress root.img.zst -> /dev/mapper/bes-target-root
cryptsetup close bes-target-root
```

The empty keyfile (`/tmp/bes-empty-keyfile`) is already created by
`encryption.rs::create_empty_keyfile`. We can reuse that or create a
parallel helper in `writer.rs`.

### Filesystem UUID rotation

All partition images extracted from the cloud image have the same filesystem
UUIDs on every install. Since the installer now writes partition images
rather than whole-disk images, every installation from the same ISO would
share identical UUIDs. This causes problems when two disks installed from
the same ISO are mounted on the same system (UUID collisions), and is
generally poor hygiene.

After writing and expanding partitions, the installer must randomize the
filesystem UUID of each partition:

- **xboot (ext4)**: `tune2fs -U random /dev/sdX2`. Safe on an unmounted
  filesystem, rewrites the superblock only.
- **root (BTRFS)**: `btrfstune -u /dev/sdX3` (or `/dev/mapper/root` if
  encrypted). Requires the filesystem to be unmounted. The `-u` flag
  changes the fsid (the `-m` flag changes the metadata UUID, but `-u` is
  the standard approach for an offline filesystem).
- **EFI (FAT32)**: FAT32 has a 32-bit volume serial number, not a UUID.
  `mlabel -n -i /dev/sdX1 ::` randomizes it. Optional but included for
  completeness.

After changing UUIDs, the GRUB config (`grub.cfg`) and the initramfs both
contain stale references to the old UUIDs. GRUB uses
`search --fs-uuid <uuid>` and the kernel command line contains
`root=UUID=<uuid>`. Dracut with `hostonly="yes"` bakes the root UUID into
the initramfs. Both must be regenerated.

### Initramfs and GRUB rebuild

The installer must unconditionally rebuild the initramfs and GRUB config
after writing partition images, regardless of encryption mode. This is
needed because:

1. Filesystem UUIDs have been rotated (see above).
2. For encrypted installs, the crypttab and keyfile changes also require
   an initramfs rebuild (previously the only reason for rebuilding).

The rebuild runs in a chroot with bind-mounted `/proc`, `/sys`, `/dev`:

```
chroot /mnt/target dracut --force --kver <kver>
chroot /mnt/target update-grub
```

Today `encryption.rs::configure_installed_system` already does the dracut
rebuild for encrypted installs. With this change, the rebuild happens
unconditionally in a new `rebuild_boot_config` function. For encrypted
installs, the encryption setup phase will rebuild dracut a second time
(after writing crypttab and keyfile changes). This double-rebuild is
intentional: the first rebuild picks up the new UUIDs, and the second
picks up the encryption config. Combining them would require reordering
the encryption setup to happen before firstboot, which would complicate
the flow for marginal benefit.

### Progress reporting and TUI screen flow

Today the TUI has separate screens for each post-write phase: `Writing`,
`FirstbootApply`, `EncryptionSetup`, `RecoveryPassphrase`, `Done`. This
means the progress bar disappears after the image write and the user sees
a sequence of brief "Applying..." screens with no progress indication.

In the new design, a single `Installing` screen with one progress bar
covers the entire installation. The progress bar is weighted so that the
partition writes (which have byte-level progress) occupy the bulk of the
bar, and each post-write step gets a 1% slice even though we only know
when it starts and finishes. This gives the user a sense of forward motion
throughout the entire process.

Approximate weight allocation:

- Partition writes (EFI + xboot + root): ~90% (proportional to bytes,
  tracked precisely via the existing `WriteProgress` mechanism).
- Expand root filesystem: 1%
- Randomize UUIDs: 1%
- Rebuild boot config (dracut + update-grub): 2%
- Verify partition table: 1%
- First-boot configuration: 1%
- Encryption setup (if encrypted): 4% (or 0% if `None`)

The exact percentages are not critical; what matters is that the bar
moves for every phase. Each post-write step jumps the bar forward by its
slice when it completes.

After all steps complete, the TUI transitions to a `Done` screen. For
encrypted installs, the `Done` screen also displays the recovery
passphrase (which was pre-generated at confirmation time and is now
enrolled). The user must press Enter to acknowledge. This replaces the
separate `RecoveryPassphrase` screen.

The `Screen` enum changes:
- `Writing` -> `Installing` (covers all phases, not just image write).
- Remove `FirstbootApply` and `EncryptionSetup` (folded into
  `Installing`).
- Remove `RecoveryPassphrase` (folded into `Done`).
- `Done` shows the recovery passphrase when encryption is enabled.

For automatic mode (`run_auto`), the same structure applies: a single
progress output covering all phases, with the recovery passphrase printed
to stderr at the end.

### Disk size check

The minimum disk size is the sum of the fixed partition sizes (512 MiB +
1 GiB) plus the uncompressed root image size, with some slack for GPT
overhead. In practice, reading `root.img.size` and adding 1.5 GiB is
sufficient. `partitions.json` could include a `min_disk_bytes` field
computed at ISO build time for extra clarity.

## Code changes

### `iso/build-iso.sh`

Replace Phase 5 ("Copy disk images into staging") with a new phase that:

1. Finds the cloud `.raw.zst` image and decompresses it to a temporary
   `.raw` file.
2. Loop-mounts the raw image with `losetup -f --show -P`.
3. Uses `dd` to extract each partition to a separate file
   (`efi.img`, `xboot.img`, `root.img`).
4. Compresses each with `zstd`.
5. Writes `.size` sidecars (output of `stat --format='%s'` on the
   uncompressed partition files).
6. Generates `partitions.json` from the image's `sgdisk --info` output.
7. Detaches the loop device and cleans up.

The ISO build no longer requires the metal image. Update the cleanup
function to handle new temporary files (raw decompressed image, loop
device).

### `justfile`

- **`iso` recipe**: Change to require only the cloud image, not both.
  Remove the check for `METAL_IMAGE`. Keep the check for `CLOUD_IMAGE`.

### `installer/tui/src/writer.rs`

- **`PartitionManifest` struct** (new): Parsed representation of
  `partitions.json`. Contains `arch: String` and `partitions:
  Vec<PartitionEntry>`. Each `PartitionEntry` has `label`, `type_uuid`,
  `size_mib`, `image` (filename).
- **`find_partition_manifest`** (new, replaces `find_image_path`): Searches
  the same directories (`/run/live/medium/images`, etc.) for
  `partitions.json`. Parses and returns a `PartitionManifest`. Also returns
  the directory path so image files can be located relative to it.
- **`partition_images_total_size`** (new, replaces
  `image_uncompressed_size`): Reads each partition's `.size` sidecar and
  returns the sum.
- **`create_partition_table`** (new): Wraps `sgdisk` calls to create the
  GPT and all three partitions from `PartitionManifest` data. For each
  partition with `size_mib > 0`, uses `-n N:0:+{size_mib}M`. For
  `size_mib == 0`, uses `-n N:0:0`. Sets type UUID with `-t` and label
  with `-c` for each.
- **`format_luks_for_root`** (new): Creates an empty keyfile, runs
  `cryptsetup luksFormat --type luks2`, opens the volume, returns the
  mapper device path. Caller is responsible for closing after write.
- **`close_luks_root`** (new): Runs `cryptsetup close`.
- **`decompress_to_device`** (new, extracted from `write_image`): Takes a
  source `.img.zst` path, a target device path, and a progress callback.
  Streams zstd-decompressed data to the device. This is the inner loop of
  the old `write_image`, factored out.
- **`write_partitions`** (new, replaces `write_image`): Orchestrates the
  full new write flow. Calls `wipe_disk`, `create_partition_table`,
  `reread_partition_table`, then `decompress_to_device` for each partition.
  For root when encryption is enabled, calls `format_luks_for_root` first
  and writes to the mapper device. Reports unified progress.
- **`expand_root_filesystem`** (new, replaces `expand_partitions`):
  Temporarily opens the LUKS volume (if encrypted) and mounts the BTRFS
  root, runs `btrfs filesystem resize max /mnt/target`, then unmounts and
  closes LUKS. If encrypted, also runs `cryptsetup resize root` before the
  BTRFS resize.
- **`randomize_filesystem_uuids`** (new): Runs `tune2fs -U random` on
  xboot, `btrfstune -u` on root (or `/dev/mapper/root` if encrypted),
  and `mlabel -n` on EFI. Must be called after `expand_root_filesystem`
  and before `rebuild_boot_config`, with all filesystems unmounted.
- **`rebuild_boot_config`** (new): Mounts the target (root, xboot, EFI),
  bind-mounts `/proc`, `/sys`, `/dev`, runs `dracut --force --kver <kver>`
  and `update-grub` in a chroot, then cleans up. Called unconditionally
  after UUID rotation.
- **`expand_partitions`**: Delete. The GPT is created with correct geometry
  so `growpart` and `--move-second-header` are unnecessary, and the BTRFS
  resize is now handled by `expand_root_filesystem`.
- **`check_disk_size`**: No signature change needed; callers pass the new
  summed size.
- **`find_image_path`**: Delete (replaced by `find_partition_manifest`).
- **`image_uncompressed_size`**: Keep as a helper but it is no longer the
  primary entry point for disk size checks.
- **`write_image`**: Delete (replaced by `write_partitions`).

### `installer/tui/src/firstboot.rs`

- **`fixup_for_metal_variant`** (new): Called after `mount_target` when
  encryption is not `None`. Performs:
  1. Rewrite `/etc/fstab`: replace `by-partlabel/root` with
     `/dev/mapper/root` on root and postgresql lines.
  2. Write `metal` to `/etc/bes/image-variant`.
  3. If no explicit hostname is configured, truncate `/etc/hostname` to
     empty (matching metal image behavior from `image.hostname.metal-dhcp`).
  4. Create `/etc/luks/empty-keyfile` with mode 000 (needed for the
     initial unlock before key rotation). Note: `encryption.rs`
     `create_empty_keyfile` creates one at `/tmp/`; we need one on the
     target filesystem at the standard path.
- **`mount_target`**: When this function is called after the new write
  flow, the LUKS volume will be closed. For encrypted variants, it must
  re-open LUKS with the empty keyfile. This already works because
  `open_luks` in `firstboot.rs` uses the empty keyfile. However, the
  empty keyfile at `/etc/luks/empty-keyfile` is on the *target* system
  which is not yet mounted. The current code creates a temporary empty
  keyfile at `/tmp/bes-empty-keyfile` for this purpose, which is fine.
  No change needed here.

### `installer/tui/src/ui.rs`

- **`Screen` enum**: Replace `Writing`, `FirstbootApply`,
  `EncryptionSetup`, `RecoveryPassphrase` with a single `Installing`
  screen. The `Done` screen now also displays the recovery passphrase
  when encryption is enabled.
- **`AppState`**: Add an `install_phase: InstallPhase` enum field to
  track which phase the install worker is in (e.g. `WritingPartitions`,
  `ExpandingRoot`, `RandomizingUuids`, `RebuildingBoot`,
  `VerifyingPartitions`, `ApplyingFirstboot`, `SettingUpEncryption`).
  Add `install_progress: f64` (0.0 to 1.0) for the unified bar.
- **`advance`**: `Confirmation` -> `Installing`. `Installing` does not
  advance (the worker sends a message when done). Worker completion
  transitions to `Done`.

### `installer/tui/src/main.rs`

- **`run_auto`**: Replace `find_image_path` + `write_image` sequence with
  `find_partition_manifest` + `write_partitions`. The entire install
  sequence (write, expand, randomize UUIDs, rebuild boot config, verify,
  firstboot, encryption) runs in order with progress printed to stderr.
  Recovery passphrase is printed at the end.
- **`run_interactive`**: Replace `find_image_path` call with
  `find_partition_manifest`. Pass manifest to `ui::run_tui`.

### `installer/tui/src/ui/run.rs`

- **`start_install_worker`** (replaces `start_write_worker` +
  `start_firstboot_worker` + `start_encryption_worker`): A single worker
  thread that runs the entire install sequence, sending
  `WorkerMessage::Phase(InstallPhase)` and
  `WorkerMessage::Progress(f64)` messages as it goes. The worker calls:
  1. `write_partitions` (with byte-level progress mapped to 0-90%).
  2. `expand_root_filesystem` (90-91%).
  3. `randomize_filesystem_uuids` (91-92%).
  4. `rebuild_boot_config` (92-94%).
  5. `verify_partition_table` (94-95%).
  6. `fixup_for_metal_variant` + `apply_firstboot` (95-96%).
  7. `run_encryption_setup` if encrypted (96-100%), or jump to 100%.
  On completion, sends `WorkerMessage::InstallDone`. On error, sends
  `WorkerMessage::InstallError(String)`.
- Delete `start_firstboot_worker` and `start_encryption_worker` (folded
  into `start_install_worker`).

### `installer/tui/src/plan.rs`

- Rename `image_path: Option<PathBuf>` to `manifest_path: Option<PathBuf>`.
  Update `InstallPlan::new` signature and all callers.
- Update dry-run schema in spec to match.

### `tests/test-iso-structure.sh`

- Remove checks for `*-metal-*` and `*-cloud-*` `.raw.zst` images.
- Add checks for:
  - `partitions.json` exists and is valid JSON.
  - `efi.img.zst`, `xboot.img.zst`, `root.img.zst` exist.
  - Corresponding `.size` sidecars exist.
  - No `.raw.zst` whole-disk images present (ensure clean migration).

### `tests/container-install-scenarios.json`

- No changes needed. Scenarios are defined by `disk-encryption` and
  firstboot config, not by image selection. The installer derives the
  variant from encryption mode as before.

## Testing

### Unit tests (`installer/tui`)

- Test `partitions.json` parsing: valid, missing fields, bad JSON,
  unknown arch.
- Test `create_partition_table` generates the expected `sgdisk` command
  arguments (capture `Command` or verify via mock).
- Test `fixup_for_metal_variant` correctly rewrites fstab entries (create
  temp file with cloud-style fstab, run fixup, verify output).
- Test `randomize_filesystem_uuids` changes UUIDs (would require real
  filesystems; may be integration-test-only).
- Test `partition_images_total_size` sums correctly.
- Test disk size check with summed partition sizes.
- Update existing `find_image_path` tests to test `find_partition_manifest`.
- Update `image_uncompressed_size` tests (still used as a helper).
- Update `InstallPlan` serialization tests to use `manifest_path` instead
  of `image_path`.

### Integration tests (container-based)

- Existing `test-container-install-all.sh` scenarios exercise the full
  installer flow against a loop device. They should work with minimal
  changes once the ISO is rebuilt with partition images.
- The test scenarios already cover `tpm`, `keyfile`, and `none` encryption
  modes, which now all start from the same cloud partition images.
- Add verification in the install test that checks `/etc/fstab` references
  `/dev/mapper/root` when encryption is enabled.
- Add verification that `/etc/bes/image-variant` is `metal` or `cloud`
  as appropriate.
- Add verification that filesystem UUIDs on the installed disk differ from
  a known set (or at minimum that `grub.cfg` references the actual
  filesystem UUID of the root partition).

### Manual verification

- Build an ISO and confirm it is roughly half the size of the old ISO.
- Boot the ISO in a VM and install with each encryption mode (TPM, keyfile,
  none). Verify the installed system boots correctly.
- Verify the installed system's fstab, crypttab, and variant marker are
  correct for each mode.
- Install two disks from the same ISO and verify they have different
  filesystem UUIDs.

## Implementation order

1. **`writer.rs` refactor** -- add `PartitionManifest`,
   `find_partition_manifest`, `decompress_to_device`, `create_partition_table`,
   `format_luks_for_root`, `write_partitions`. Keep old functions temporarily
   for compilation.
2. **`firstboot.rs` additions** -- add `fixup_for_metal_variant`.
3. **`main.rs` / `ui/run.rs` rewiring** -- switch `run_auto` and
   `start_write_worker` to the new flow.
4. **`plan.rs`** -- rename `image_path` to `manifest_path`.
5. **`iso/build-iso.sh`** -- replace Phase 5 with partition extraction.
6. **`justfile`** -- relax `iso` recipe to require only cloud image.
7. **`test-iso-structure.sh`** -- update checks.
8. **Delete old code** -- remove `find_image_path`, `write_image`,
   `expand_partitions`.
9. **Run full test suite** -- `cargo clippy`, `cargo fmt`, `tracey query
   status`, then container install tests.

## Resolved decisions

- **No schema version in `partitions.json`.** The installer binary and the
  ISO are always built together in the same pipeline. There is no scenario
  where they drift apart, so versioning the manifest format adds complexity
  for no benefit.
- **All three partition images are zstd-compressed.** EFI is mostly empty
  (just a GRUB binary and config), xboot has a kernel and initramfs, and
  both compress extremely well. Keeping them compressed avoids special-casing
  the write path and saves over 1 GiB of ISO space for no extra complexity.
- **Raw `dd` extraction, not `btrfs send`.** The root partition is ~3.5 GiB
  uncompressed but mostly unallocated BTRFS space, which zstd compresses to
  nearly nothing. A `btrfs send` stream would save marginal space while
  requiring `mkfs.btrfs` + `btrfs receive` on the target (different write
  path, subvolume snapshot semantics to manage). Not worth the complexity.
- **Filesystem UUIDs are rotated unconditionally.** Even though cloud
  installs (encryption `None`) don't strictly need it today, UUID
  uniqueness is basic hygiene and avoids problems if two disks from the
  same ISO are ever mounted together. The cost is an unconditional
  initramfs + GRUB rebuild (~10-15 seconds).
- **Initramfs is rebuilt twice for encrypted installs.** Once after UUID
  rotation (unconditional), and once after encryption setup (to pick up
  crypttab/keyfile). Merging them would require reordering the phases,
  which is not worth the complexity. The second rebuild is fast since
  dracut caches module resolution.
- **LUKS mapper name is `bes-target-root`** (matching the existing constant
  `LUKS_NAME` in `firstboot.rs`). Both the new `format_luks_for_root` in
  `writer.rs` and the existing `mount_target` in `firstboot.rs` use this
  name.
- **The cloud image's hostname (`ubuntu`) is overwritten for metal.**
  When encryption is enabled and no explicit hostname is configured, the
  fixup truncates `/etc/hostname` to match `image.hostname.metal-dhcp`.
  When a hostname IS configured, `apply_hostname` in firstboot already
  writes it, overriding the cloud default.

## Migration notes

- Old ISOs with whole-disk `.raw.zst` images will not work with the new
  installer, and new ISOs will not work with the old installer. This is
  acceptable since the ISO and installer are always built together.
- The standalone metal and cloud `.raw.zst` images are unchanged and remain
  compatible with direct `dd`-to-disk workflows.
- `bes-install.toml` files do not need changes.
- The `variant` field in the dry-run schema is still derived from
  `disk_encryption`; nothing changes for consumers of the plan JSON except
  the `image_path` -> `manifest_path` rename.
