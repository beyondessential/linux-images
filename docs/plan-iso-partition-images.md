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

This plan assumes the disk encryption rework (see
`plan-disk-encryption-rework.md`) is complete. In particular, it assumes:

- The installer already has a `DiskEncryption` enum (`TpmBound`, `Keyfile`,
  `None`) and derives the variant from it.
- The installer already performs all LUKS setup (key rotation, TPM/keyfile
  enrollment, recovery passphrase, empty-slot wipe, initramfs rebuild).
- The metal image no longer contains any first-boot TPM enrollment services.

## Current state

The ISO embeds both `metal-<arch>.raw.zst` and `cloud-<arch>.raw.zst`. These
are complete GPT disk images (EFI + xboot + root). The installer picks one
based on the user's variant selection, decompresses it as a whole-disk stream
to the target device, then expands partitions to fill the disk.

The metal and cloud images differ only in:

1. The root partition: metal wraps BTRFS in LUKS2; cloud has bare BTRFS.
2. A handful of config files: `/etc/crypttab`, `/etc/fstab`,
   `/etc/bes/image-variant`, `/etc/luks/empty-keyfile`, dracut LUKS config.

This means the ISO ships approximately 2x the compressed data for what is
essentially the same OS content.

## Design

### ISO contents (new)

The ISO will contain three compressed partition images extracted from the
cloud image, plus metadata:

```
/images/efi.img.zst          # FAT32 EFI System Partition (512 MiB uncompressed)
/images/efi.img.size          # uncompressed byte count
/images/xboot.img.zst         # ext4 extended boot partition (1 GiB uncompressed)
/images/xboot.img.size
/images/root.img.zst          # bare BTRFS root partition (~3.5 GiB uncompressed)
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

Currently:

1. Wipe disk.
2. Stream-decompress whole `.raw.zst` to block device.
3. Re-read partition table.
4. Expand partitions.

New flow:

1. Wipe disk.
2. Read `partitions.json`.
3. Create GPT and partitions via `sgdisk`.
4. Re-read partition table.
5. Write EFI partition: decompress `efi.img.zst` -> `/dev/sdX1`.
6. Write xboot partition: decompress `xboot.img.zst` -> `/dev/sdX2`.
7. Write root partition:
   - If encryption is `None`: decompress `root.img.zst` -> `/dev/sdX3`.
   - If encryption is `TpmBound` or `Keyfile`:
     a. `cryptsetup luksFormat --type luks2` on `/dev/sdX3` with empty
        passphrase.
     b. `cryptsetup open` -> `/dev/mapper/root`.
     c. Decompress `root.img.zst` -> `/dev/mapper/root`.
     d. Close LUKS (it will be reopened by the encryption setup phase).
8. Expand root partition to fill disk (same `growpart` logic as today, but
   now the partition was created at the right size so this is only needed
   for the BTRFS resize at boot).

Steps 5-7 reuse the existing `zstd::Decoder` streaming logic from
`write_image`, factored out to accept a source path and a target device path.

### Config fixups

The cloud image's root filesystem has cloud-style config. When encryption is
enabled, the installer must patch the following files during the firstboot
mount phase (which already exists):

- **`/etc/fstab`**: Replace `by-partlabel/root` references with
  `/dev/mapper/root` for the root and postgresql mount entries.
- **`/etc/crypttab`**: Create the file with the appropriate entry (device
  `by-partlabel/root`, keyfile and options depend on encryption mode -- this
  is already handled by `installer.encryption.configure-system`).
- **`/etc/bes/image-variant`**: Write `metal` (already planned: the
  installer derives variant from `DiskEncryption` and writes it).
- **`/etc/luks/empty-keyfile`**: Create with mode 000 (needed for the
  initial unlock before key rotation -- the encryption setup phase already
  handles this).
- **Dracut LUKS config**: Install the keyfile include directive so the
  initramfs can unlock at boot (already part of
  `installer.encryption.configure-system`).
- **Initramfs rebuild**: Run `dracut` in a chroot (already part of
  `installer.encryption.configure-system`).

When encryption is `None`, no fixups are needed -- the cloud image's fstab
and lack of crypttab are already correct.

### Progress reporting

EFI (~512 MiB) and xboot (~1 GiB) are small relative to root (~3.5 GiB).
Show a single unified progress bar across all three writes. Sum the three
`.size` sidecars for the total, and accumulate bytes written across all
three streams.

### Disk size check

The minimum disk size is the sum of the fixed partition sizes (512 MiB +
1 GiB) plus the uncompressed root image size, with some slack for GPT
overhead. In practice, reading `root.img.size` and adding 1.5 GiB is
sufficient. Alternatively, `partitions.json` could include a
`min_disk_bytes` field computed at ISO build time.

## Spec changes

### `docs/spec/live-iso.md`

- **`iso.contents`**: Replace "compressed disk images for all variants" with
  "compressed partition images extracted from the cloud disk image". Describe
  the `partitions.json` manifest and `.size` sidecars.
- Remove references to selecting between metal/cloud images on the ISO.

### `docs/spec/installer.md`

- **`installer.write.source`**: The installer reads `partitions.json` from
  the ISO filesystem rather than searching for a `.raw.zst` by variant name.
  There is one set of partition images per architecture, not per variant.
- **`installer.write.partitions`**: After wiping the disk, the installer
  creates the GPT table and all three partitions using the geometry from
  `partitions.json`. Partition type UUIDs, labels, and sizes match the
  original image spec.
- **`installer.write.decompress-stream`**: The installer stream-decompresses
  each partition image to its corresponding partition device (or to the
  opened LUKS device for the root partition when encryption is enabled).
- **`installer.write.disk-size-check`**: The check uses the sum of partition
  image sizes rather than a single whole-disk image size.
- **`installer.write.luks-before-write`**: (New) When encryption is not
  `None`, the installer must format the root partition with LUKS2 and open
  it before writing the root partition image. This is the initial LUKS
  setup with an empty passphrase; key rotation and mechanism enrollment
  happen in the subsequent `installer.encryption.*` phase.
- **`installer.write.fstab-fixup`**: (New) When encryption is not `None`,
  the installer must rewrite `/etc/fstab` on the installed system to
  reference `/dev/mapper/root` instead of `/dev/disk/by-partlabel/root`
  for the root and postgresql mounts.

### `docs/spec/disk-images.md`

No changes. Both metal and cloud images are still built and published. The
metal image remains available for direct-imaging workflows outside the
installer.

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

The ISO build no longer requires the metal image to be present. The
`justfile` `iso` recipe should be updated to require only the cloud image.

### `installer/tui/src/writer.rs`

- **`find_image_path`**: Replace with `find_partition_manifest` that locates
  and parses `partitions.json` from the ISO filesystem. Returns a struct
  with paths to each partition image file, their sizes, and GPT metadata.
- **`write_image`**: Replace with `write_partitions` that:
  1. Calls `wipe_disk`.
  2. Calls a new `create_partition_table` function (wraps `sgdisk`).
  3. Calls `reread_partition_table`.
  4. Streams each partition image to its device. For the root partition,
     if encryption is enabled, sets up LUKS first and writes to the mapper
     device.
  5. Reports unified progress across all three writes.
- **`image_uncompressed_size`**: Adapt to work with per-partition `.size`
  files and provide a combined total for the disk size check.
- **`expand_partitions`**: Simplify. The GPT was created by the installer
  with partition 3 already spanning all remaining space, so `growpart` is
  unnecessary. The only remaining job is ensuring the GPT secondary header
  is at the end of the disk (it will be, since `sgdisk` placed it there).
  The BTRFS `resize max` still happens at boot via `grow-root-filesystem`.
- **`check_disk_size`**: Accept the sum of partition sizes plus overhead.

### `installer/tui/src/firstboot.rs`

- **`mount_target`**: When encryption is not `None`, the LUKS volume may
  already be closed after the partition write phase. The encryption setup
  phase (from the disk encryption rework) reopens it. This function's
  existing logic for opening LUKS with the empty keyfile remains correct.
- Add `fixup_fstab_for_encryption`: Reads `/etc/fstab`, replaces
  `by-partlabel/root` with `/dev/mapper/root` on the root and postgresql
  lines. Called after mounting the target when encryption is enabled. (The
  crypttab, keyfile, dracut config, and initramfs rebuild are already
  handled by the encryption setup phase.)

### `installer/tui/src/main.rs`

- Update `run_auto` and `run_interactive` to call the new `write_partitions`
  instead of `write_image`, passing the `DiskEncryption` mode.
- The fstab fixup is called in the firstboot phase, gated on encryption
  being enabled.

### `installer/tui/src/plan.rs`

- Remove `image_path` from `InstallPlan` (there is no longer a single image
  path; the manifest path could replace it if needed for diagnostics).

### `justfile`

- Update the `iso` recipe to require only the cloud image, not both.
- Consider adding a `partition-images` recipe for debugging that extracts
  partition images without building a full ISO.

## Testing

### Unit tests (`installer/tui`)

- Test `partitions.json` parsing (valid, missing fields, bad JSON).
- Test `create_partition_table` generates the expected `sgdisk` commands
  (mock or capture).
- Test `fixup_fstab_for_encryption` correctly rewrites fstab entries.
- Test disk size check with summed partition sizes.
- Update existing `write_image` tests to cover the new `write_partitions`
  flow.

### Integration tests

- Existing container-based install tests (`test-container-install`) should
  work with minimal changes -- they exercise the full installer flow against
  a loop device.
- Add a test that verifies the ISO contains `partitions.json` and the
  expected partition image files (no `.raw.zst` whole-disk images).
- Add a test that installs with encryption enabled and verifies the fstab
  contains `/dev/mapper/root` references.

### Manual verification

- Build an ISO and confirm it is roughly half the size of the old ISO.
- Boot the ISO in a VM and install with each encryption mode (TPM, keyfile,
  none). Verify the installed system boots correctly.

## Migration notes

- Old ISOs with whole-disk `.raw.zst` images will not work with the new
  installer, and new ISOs will not work with the old installer. This is
  acceptable since the ISO and installer are always built together.
- The standalone metal and cloud `.raw.zst` images are unchanged and remain
  compatible with direct `dd`-to-disk workflows.
- `bes-install.toml` files do not need changes.

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