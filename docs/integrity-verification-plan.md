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

### Self-Describing Verity Layout

All verity-protected blobs use the same binary layout:

```
[data (N bytes)] [verity hash tree (M bytes)] [M as u64 LE (8 bytes)]
```

At runtime, the consumer:

1. Reads the last 8 bytes to recover `M` (the hash tree size).
2. Computes `hash_offset = total_size - 8 - M`.
3. Calls `veritysetup open <dev> <name> <dev> <roothash> --hash-offset=<hash_offset>`.

The data starts at offset 0, so the dm-verity device exposes a clean data
image (squashfs in both cases) that can be mounted directly.

No external metadata file is needed for offsets. The root hash is the only
piece of information stored externally — it is the trust anchor and belongs
in the GRUB kernel command line.

This layout was validated experimentally: `veritysetup` ignores trailing
bytes beyond the hash tree, so the 8-byte size trailer does not interfere.

### New ISO Layout

| GPT partition | Content | Type |
|---------------|---------|------|
| 1 (implicit)  | ISO 9660 data (kernel, initrd, squashfs with verity trailer, GRUB config) | — |
| 2             | ESP (FAT32, GRUB EFI binary + grub.cfg) | EFI System Partition |
| 3             | BESCONF (FAT32, writable, user config) | Microsoft basic data |
| 4             | Images squashfs with verity trailer | Linux filesystem data |

### Component 1: Squashfs Rootfs Verity

At build time, after `mksquashfs` produces `filesystem.squashfs`:

1. Run `veritysetup format filesystem.squashfs filesystem.squashfs.hashtree`
   to produce a hash tree file and a root hash.
2. Append the hash tree to `filesystem.squashfs`.
3. Append the hash tree file size as a little-endian u64 (8 bytes).
4. Place the resulting single file at `/live/filesystem.squashfs` in the ISO.
5. Embed the root hash in the kernel command line as
   `live.verity.roothash=<hex>`.

At boot time, a custom initramfs premount script runs before `live-boot`
mounts the squashfs:

1. Read the `live.verity.roothash=` parameter from `/proc/cmdline`.
2. Read the last 8 bytes of `/live/filesystem.squashfs` to recover
   `hash_size`, compute `hash_offset = file_size - 8 - hash_size`.
3. Set up a loop device on the file.
4. Run `veritysetup open` with the loop device as both data and hash device,
   passing `--hash-offset=<hash_offset>`.
5. Mount the resulting `/dev/mapper/live-verity` as the squashfs root
   instead of mounting the file directly.

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

2. Run `veritysetup format images.squashfs images.squashfs.hashtree` to
   produce the hash tree and root hash.

3. Append the hash tree to the squashfs:
   ```
   cat images.squashfs.hashtree >> images.squashfs
   ```

4. Append the hash tree file size as a little-endian u64 (8 bytes).

5. Append the combined blob as GPT partition 4 via xorriso
   `--append_partition`.

6. Store the root hash in the kernel command line as
   `images.verity.roothash=<hex>`.

At runtime, the installer:

1. Finds the images partition (partition 4 of the boot device, or by
   filesystem label `BESIMAGES`).
2. Reads the last 8 bytes to recover the hash tree size, computes the
   hash offset.
3. Reads the root hash from the `images.verity.roothash=` kernel command
   line parameter.
4. Runs `veritysetup open` on the partition with `--hash-offset` and the
   root hash.
5. Mounts the dm-verity device as squashfs.
6. Reads partition images as regular files. The kernel handles both verity
   verification and zstd decompression transparently on each read.
7. Streams the raw image data directly to the target partition devices
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
   (already present in the live environment for LUKS operations). The
   installer reads the last 8 bytes of the partition to compute the hash
   offset, and reads the root hash from `/proc/cmdline`.

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
   staging directory with `-comp zstd`. Run `veritysetup format`. Append
   hash tree + size trailer.

3. **New phase: Build squashfs verity**: After creating
   `filesystem.squashfs`, run `veritysetup format`. Append hash tree +
   size trailer to the squashfs file.

4. **Phase (GRUB config)**: Add `live.verity.roothash=<hash>` and
   `images.verity.roothash=<hash>` to the kernel command line in
   `grub.cfg`.

5. **Phase (xorriso)**: Add `--append_partition 4` for the images
   partition blob.

6. **New build dependency**: `cryptsetup` (for `veritysetup`).

### Changes to the Live Rootfs

1. **New initramfs hook**: A script in
   `/usr/share/initramfs-tools/hooks/verity` that copies `veritysetup` and
   its dependencies into the initramfs.

2. **New live-boot premount script**: A script in
   `/usr/share/initramfs-tools/scripts/live-premount/` that intercepts the
   squashfs mount and wraps it with dm-verity, including the trailer read
   to recover the hash offset.

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
- `installer.write.source+2` -> `+4` in `installer/tui/src/writer/manifest.rs`,
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
- Append hash tree + size trailer to produce the self-describing blob.
- Append as GPT partition 4.
- Add `images.verity.roothash=<hash>` to the GRUB kernel command line.
- Update `r[impl iso.contents]` and `r[impl iso.minimal]` annotations.

Status: not started.

### Step 3: Squashfs verity in `build-iso.sh`

- Run `veritysetup format` on `filesystem.squashfs`.
- Append hash tree + size trailer to produce the self-describing blob.
- Add `live.verity.roothash=<hash>` to GRUB command line.

Status: not started.

### Step 4: Initramfs verity hook

- Write the initramfs hook and premount script.
- Install them in the live rootfs during Phase 2 of `build-iso.sh`.
- The premount script reads the 8-byte trailer to recover the hash offset.
- Test by booting the ISO in QEMU with a known-good image and verifying
  that dm-verity devices appear.

Status: not started.

### Step 5: Installer changes

- Add images partition discovery and verity open/mount.
- The installer reads the 8-byte trailer from the partition to compute
  the hash offset, and reads the root hash from `/proc/cmdline`.
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

## Resolved Questions

1. **Verity offset storage**: Resolved by the self-describing
   `[data | hash | size_le64]` layout. The 8-byte trailer eliminates the
   need for an external metadata file (`/images-verity.json`). Validated
   experimentally: `veritysetup` ignores trailing bytes beyond the hash
   tree.

2. **GPT type UUID for the images partition**: Using the generic Linux
   filesystem GUID via xorriso `--append_partition`. Discovery is by
   filesystem label `BESIMAGES` (via `/dev/disk/by-label/`), not by type
   UUID.

## Open Questions

1. **Failure UX for images verity**: If the images partition fails verity
   verification, the installer should show a clear error message explaining
   that the USB media is corrupted. Need to decide on exact wording and
   whether to offer a retry or just halt.

2. **Development builds**: Should there be an option to build ISOs without
   verity for faster iteration? A `SKIP_VERITY=1` environment variable in
   `build-iso.sh` could skip the verity steps and produce an ISO with plain
   squashfs (no hash trees, no root hash in cmdline). The initramfs hook
   already handles the absent-root-hash case gracefully.