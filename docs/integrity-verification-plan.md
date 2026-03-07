# Integrity Verification Plan

## Problem

The ISO image has no built-in integrity checking. ISO 9660 relies on
sector-level ECC on optical media, but when written to USB via `dd` there is
no error detection at all. A single bit flip during download, write, or from
flash wear can cause mysterious boot failures or, worse, silently corrupted
installations.

## Goals

1. The live environment can verify its own rootfs (squashfs) on every block
   read, using dm-verity. Corruption is detected at read time, not after the
   fact.

2. The partition images written to the target disk are served from a
   verity-protected filesystem, so any corruption in the install payload is
   caught before it reaches the target.

3. The installer no longer needs userspace zstd decompression. The partition
   images are stored as raw files inside a squashfs with transparent zstd
   compression; the kernel decompresses on read.

## Non-Goals

- Secure Boot chain of trust (signing the root hash, tying into UEFI
  Secure Boot). This plan covers corruption detection, not authenticity.
  Secure Boot integration can be layered on later.

- Write verification of the USB media itself. External tools (e.g.
  `dd` + `cmp`, Etcher's verify step) handle that.

## Background: Current ISO Layout

The ISO produced by `build-iso.sh` is a hybrid image: simultaneously a valid
ISO 9660 filesystem and a GPT disk. After `dd` to USB, the GPT contains:

| GPT partition | Content | Type |
|---------------|---------|------|
| 1 (implicit)  | ISO 9660 data (kernel, initrd, squashfs, partition images, GRUB config) | — |
| 2             | ESP (FAT32, GRUB EFI binary + grub.cfg) | EFI System Partition |
| 3             | BESCONF (FAT32, writable, user config) | Microsoft basic data |

The squashfs at `/live/filesystem.squashfs` contains the full live rootfs.
The partition images (`efi.img.zst`, `xboot.img.zst`, `root.img.zst`) are
zstd-compressed files with `.size` sidecars, read by the installer from
`/run/live/medium/images/`.

## Design

### New ISO Layout

| GPT partition | Content | Type |
|---------------|---------|------|
| 1 (implicit)  | ISO 9660 data (kernel, initrd, squashfs + verity hash tree, GRUB config) | — |
| 2             | ESP (FAT32, GRUB EFI binary + grub.cfg) | EFI System Partition |
| 3             | BESCONF (FAT32, writable, user config) | Microsoft basic data |
| 4             | Images squashfs + appended verity hash tree | Linux filesystem data |

### Component 1: Squashfs Rootfs Verity

At build time, after `mksquashfs` produces `filesystem.squashfs`:

1. Run `veritysetup format filesystem.squashfs filesystem.squashfs.verity`
   to produce a hash tree file and a root hash.
2. Place both `filesystem.squashfs` and `filesystem.squashfs.verity` inside
   the ISO at `/live/`.
3. Embed the root hash in the kernel command line as
   `live.verity.roothash=<hex>`.

At boot time, a custom initramfs hook (a `live-boot` hook script) runs
before the squashfs is mounted:

1. Read the `live.verity.roothash=` parameter from `/proc/cmdline`.
2. Set up a loop device on `/live/filesystem.squashfs`.
3. Run `veritysetup open` with the loop device as the data device and
   `/live/filesystem.squashfs.verity` as the hash device.
4. Mount the resulting `/dev/mapper/live-verity` as the squashfs root
   instead of mounting the squashfs file directly.

If the root hash parameter is absent, the hook is skipped and boot proceeds
without verification (graceful fallback for development builds).

If verification fails on any block read, the kernel returns I/O errors for
that block. `live-boot` will fail to mount the root or individual reads will
fail at runtime, which is the desired behavior: do not silently use
corrupted data.

### Component 2: Images Partition with Verity

At build time, instead of placing compressed `.img.zst` files inside the
ISO 9660 filesystem:

1. Pack the raw (uncompressed) partition images and `partitions.json` into a
   squashfs with zstd compression:
   ```
   mksquashfs images-dir/ images.squashfs -comp zstd
   ```
   This replaces the per-file zstd compression. The squashfs handles
   compression transparently; files appear uncompressed when mounted.

2. Run `veritysetup format images.squashfs images.squashfs.verity` to
   produce the hash tree and root hash.

3. Concatenate the hash tree onto the squashfs:
   ```
   cat images.squashfs.verity >> images.squashfs
   ```
   Record the byte offset where the hash tree starts (= original squashfs
   size). This is the `--hash-offset` for `veritysetup open`.

4. Append the combined blob as GPT partition 4 via xorriso
   `--append_partition`.

5. Store the root hash and hash offset in a metadata file inside the ISO
   (e.g. `/images-verity.json`), and/or in the kernel command line.

At runtime, the installer:

1. Finds the images partition (partition 4 of the boot device, or by GPT
   type UUID / label).
2. Runs `veritysetup open` on the partition with `--hash-offset` and the
   stored root hash.
3. Mounts the dm-verity device as squashfs.
4. Reads partition images as regular files. The kernel handles both verity
   verification and zstd decompression transparently on each read.
5. Streams the raw image data directly to the target partition devices
   (same `dd`-style write loop, but reading uncompressed data from the
   mounted squashfs instead of piping through a zstd decoder).

### Changes to `partitions.json`

The manifest format changes:

- `image` field values change from `"efi.img.zst"` to `"efi.img"` (raw
  files, no `.zst` suffix).
- The `.size` sidecar files are no longer needed. The mounted squashfs
  exposes the real file sizes via `stat`.

### Changes to the Installer

1. **Remove zstd decompression**: The `decompress_to_device` method is
   replaced with a plain streaming copy (`copy_to_device`). No `zstd`
   crate dependency needed for image writing.

2. **Remove `.size` sidecar logic**: `image_uncompressed_size()` is replaced
   by `std::fs::metadata().len()` on the mounted file.

3. **Add verity setup**: Before reading images, the installer must open the
   images partition via `veritysetup open`. This requires `cryptsetup`
   (already present in the live environment for LUKS operations).

4. **Image source discovery**: Instead of searching for `partitions.json`
   under `/run/live/medium/images/`, the installer searches for the
   images verity partition, opens it, mounts it, and reads from the mount
   point. A fallback path for development/testing (pre-mounted directory)
   should be preserved.

5. **Disk size check**: Uses `stat` on the raw `.img` files instead of
   reading `.size` sidecars.

### Changes to `build-iso.sh`

1. **Phase 5 (partition image extraction)**: Stop compressing with zstd.
   Extract raw partition images and place them in a staging directory
   alongside `partitions.json`.

2. **New phase: Build images squashfs**: Run `mksquashfs` on the images
   staging directory with `-comp zstd`. Run `veritysetup format`. Record
   root hash and hash offset.

3. **New phase: Build squashfs verity**: After creating
   `filesystem.squashfs`, run `veritysetup format` and place the hash tree
   file alongside it in the ISO staging area.

4. **Phase (GRUB config)**: Add `live.verity.roothash=<hash>` to the kernel
   command line in `grub.cfg`.

5. **Phase (xorriso)**: Add `--append_partition 4` for the images squashfs
   (with appended verity hash tree).

6. **New build dependency**: `cryptsetup` (for `veritysetup`).

### Changes to the Live Rootfs

1. **New initramfs hook**: A script in
   `/usr/share/initramfs-tools/hooks/verity` that copies `veritysetup` and
   its dependencies into the initramfs.

2. **New live-boot premount script**: A script in
   `/usr/share/initramfs-tools/scripts/live-premount/` that intercepts the
   squashfs mount and wraps it with dm-verity.

3. **Package dependency**: `cryptsetup` must be installed in the live rootfs
   (it is already present for LUKS operations during installation).

### Changes to Testing

1. **Container tests**: The bind-mount of images into the container changes
   from `--bind-ro=$images_dir:/run/live/medium/images` to either mounting
   a pre-built images squashfs or bind-mounting a plain directory as a
   fallback (for tests that do not need verity verification).

2. **New test**: Verify that corrupting a byte in the images squashfs causes
   `veritysetup verify` to fail.

3. **New test**: Verify that corrupting a byte in `filesystem.squashfs`
   causes `veritysetup verify` to fail.

## Implementation Order

Each step is a commit (or small group of commits).

### Step 1: Spec updates

Update `docs/spec/live-iso.md` and `docs/spec/installer.md` with new
requirements for verity, the images partition, and the removal of zstd
streaming decompression.

Status: done.

The following tracey references are now stale and will be resolved during
implementation steps 2-6:

- `iso.minimal+2` -> `+3` in `iso/build-iso.sh` (step 2)
- `iso.contents+2` -> `+3` in `iso/build-iso.sh`, `tests/test-iso-structure.sh` (steps 2, 6)
- `installer.write.source+2` -> `+3` in `installer/tui/src/writer/manifest.rs`,
  `installer/tui/tests/` (step 5)
- `installer.write.disk-size-check+2` -> `+3` in `installer/tui/src/writer/manifest.rs`,
  `installer/tui/src/run.rs`, `installer/tui/src/ui/run.rs`,
  `tests/test-iso-structure.sh` (steps 5, 6)
- `installer.write.decompress-stream+2` removed, replaced by
  `installer.write.stream-copy` in `installer/tui/src/writer/disk_writer.rs`,
  `installer/tui/src/writer/progress.rs` (step 5)

### Step 2: Images squashfs in `build-iso.sh`

- Stop zstd-compressing individual partition images.
- Build a squashfs containing raw images + `partitions.json`.
- Run `veritysetup format` on the images squashfs.
- Append as GPT partition 4 with verity hash tree.
- Write root hash + hash offset to `/images-verity.json` in the ISO.
- Update `r[impl iso.contents]` and `r[impl iso.minimal]` annotations.

Status: not started.

### Step 3: Squashfs verity in `build-iso.sh`

- Run `veritysetup format` on `filesystem.squashfs`.
- Place hash tree at `/live/filesystem.squashfs.verity`.
- Add `live.verity.roothash=<hash>` to GRUB command line.

Status: not started.

### Step 4: Initramfs verity hook

- Write the initramfs hook and premount script.
- Install them in the live rootfs during Phase 2 of `build-iso.sh`.
- Test by booting the ISO in QEMU with a known-good image and verifying
  that dm-verity devices appear.

Status: not started.

### Step 5: Installer changes

- Add images partition discovery and verity open/mount.
- Replace `decompress_to_device` with `copy_to_device`.
- Remove `.size` sidecar logic; use `stat` for sizes.
- Remove `partitions.json` `image` field `.zst` suffix.
- Update `find_partition_manifest` search paths.
- Preserve fallback for plain directory (testing).
- Update all stale `installer.write.source`, `installer.write.disk-size-check`,
  and `installer.write.decompress-stream` annotations.

Status: not started.

### Step 6: Test updates

- Update container test harness for the new images layout.
- Add verity corruption tests.
- Verify ISO boots in QEMU with verity enabled.
- Update stale annotations in `tests/test-iso-structure.sh`.

Status: not started.

### Step 7: Cleanup

- Remove unused zstd decompression code from the installer.
- Remove the `zstd` crate dependency if no longer needed elsewhere.
- Update documentation and comments.
- Run `tracey query stale` and confirm zero stale references.

Status: not started.

## Open Questions

1. **GPT type UUID for the images partition**: Use the generic Linux
   filesystem GUID (`0FC63DAF-8483-4772-8E79-3D69D8477DE4`) or define a
   custom one? A custom UUID makes discovery unambiguous. A label
   (`BESIMAGES`) would also work for lookup via `/dev/disk/by-label/`.

2. **Failure UX for images verity**: If the images partition fails verity
   verification, the installer should show a clear error message explaining
   that the USB media is corrupted. Need to decide on exact wording and
   whether to offer a retry or just halt.

3. **Development builds**: Should there be an option to build ISOs without
   verity for faster iteration? A `SKIP_VERITY=1` environment variable in
   `build-iso.sh` could skip the verity steps and produce an ISO with plain
   squashfs (no hash trees, no root hash in cmdline). The initramfs hook
   already handles the absent-root-hash case gracefully.