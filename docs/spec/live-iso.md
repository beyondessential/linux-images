# Live ISO

r[iso.format]
The live ISO must be a valid ISO9660 image produced by `xorriso`. It must be
bootable when attached as optical media in virtual machines (VirtualBox, QEMU)
and when written to USB media via `dd`.

r[iso.hybrid]
The ISO must be a hybrid image: simultaneously a valid ISO9660 filesystem
(for VMs and optical media) and a valid GPT disk (for USB boot after `dd`).
`xorriso` must embed a GPT via `--efi-boot-part --efi-boot-image` and include
an EFI System Partition image for El Torito EFI boot.

r[iso.base+2]
The live rootfs must be built with `debootstrap` using the default variant
(not `--variant=minbase` or `live-build`) for consistency with the disk
image builder. The default variant provides a functional Ubuntu base
including systemd, networking (netplan, systemd-networkd, systemd-resolved),
and standard tools, reducing the amount of manual package installation
needed. The rootfs is packaged as a squashfs and placed inside the ISO.

r[iso.live-boot]
The live environment must include the `live-boot` and `live-boot-initramfs-tools`
packages so that the kernel can locate and mount the squashfs root via the
`boot=live` parameter. The squashfs must be placed at `/live/filesystem.squashfs`
inside the ISO, which is the default path `live-boot` searches.

r[iso.minimal+3]
The live environment must include a kernel, an initramfs, and enough
userspace to run the TUI installer (block device utilities and cryptsetup
for LUKS and dm-verity operations). The default debootstrap variant provides
the base; only packages not included in it need to be installed explicitly.

r[iso.blacklist-drm]
The live environment must blacklist all DRM/GPU kernel modules via
`/etc/modprobe.d/blacklist-gpu.conf`. The TUI installer runs on a text
console and needs only the EFI framebuffer (`efifb`/`simplefb`); loading
hardware-specific DRM drivers wastes time and produces spurious errors
(e.g. `vmwgfx` failing under VirtualBox). The blacklist must cover at
least: `vmwgfx`, `qxl`, `bochs`, `cirrus-qemu`, `vboxvideo`, `virtio-gpu`,
`ast`, `mgag200`, `hibmc-drm`, `hyperv_drm`, `nouveau`, `i915`, `xe`,
`amdgpu`, `radeon`, and `drm_vram_helper`. The file must use
`install <module> /bin/false` directives rather than plain `blacklist`
lines, because `blacklist` only prevents autoloading and does not prevent
transitive loading by other modules.

r[iso.network-tools+3]
The live environment must include `curl` (for HTTPS connectivity checks and
GitHub SSH key lookups) and `tailscale` (for running `tailscale netcheck`
diagnostics during installation). These are used by the interactive TUI
screens for network checks and are not required for offline installation.
The default debootstrap variant already provides `iproute2` (for `ip`) and
`iputils-ping` (for `ping`) so that network problems can be debugged
from the debug shell.

r[iso.network-config+2]
The live environment must configure automatic DHCP on all Ethernet
interfaces so that network connectivity is available without manual
setup. This must use a netplan configuration file matching `en*` with
`dhcp4: true`. The default debootstrap variant provides `netplan.io`,
`systemd-networkd`, and `systemd-resolved`.

r[iso.offline]
The live ISO must be fully functional without network connectivity. No
packages or data are downloaded during the installation process.

## Build Cleanup

r[iso.contents+3]
The ISO must contain a TUI installer binary and a `partitions.json` manifest
describing the partition layout. There is one set of partition images per
architecture, not per variant. The partition images (`efi.img`, `xboot.img`,
`root.img`) are stored as raw (uncompressed) files inside a dedicated
squashfs with transparent zstd compression (see `r[iso.images-partition+3]`).
The installer reconstructs the GPT and writes each partition individually,
setting up LUKS on the root partition when encryption is enabled.

r[iso.boot.uefi]
The ISO must be UEFI-bootable via an El Torito EFI boot catalog. The EFI
boot image is a FAT32 filesystem image containing a GRUB EFI binary at
the default removable media path (`EFI/BOOT/BOOTX64.EFI` for amd64,
`EFI/BOOT/BOOTAA64.EFI` for arm64).

r[iso.boot.autostart+4]
On boot, the live environment must automatically launch the TUI installer
and switch the active virtual terminal so the user sees it immediately.
The `reboot` command must be functional in the live environment so the
installer's reboot action works.

> r[iso.config-partition+4]
> The ISO must include an appended FAT32 partition (GPT type `Microsoft basic
> data`) created via `xorriso --append_partition`. This partition is embedded
> in the ISO file and becomes a real writable GPT partition after the ISO is
> written to USB via `dd`. Its filesystem label must be `BESCONF`. The
> partition must have a well-known GPT PARTUUID of
> `e2bac42b-03a7-5048-b8f5-3f6d22100e77` so that the installer can locate it
> via `/dev/disk/by-partuuid/` without depending on a specific partition
> number or risking label collisions with other disks. After `xorriso`
> produces the ISO, the build script must use `sfdisk --part-uuid` to stamp
> this PARTUUID onto the BESCONF partition.
>
> The installer is responsible for mounting the BESCONF partition at
> `/run/besconf`. It must locate the partition by its well-known PARTUUID,
> mount it read-only, then attempt a read-write remount to determine
> writability. No systemd mount/automount units are used; the installer
> owns the entire mount lifecycle and unmounts on exit.
>
> When booted from USB, this partition is writable and is the intended location
> for users to place a `bes-install.toml` configuration file before booting.
> When booted as optical media in a VM, the partition is read-only.

r[iso.per-arch]
Separate ISO images must be produced per architecture (amd64, arm64). Each
ISO contains only the images and installer binary for its architecture.

r[iso.usb]
The ISO must be writable to USB media using `dd` and must boot correctly on
UEFI systems from that media.

r[iso.vdi]
The build system must provide a recipe to convert the hybrid ISO to a VDI
(VirtualBox Disk Image) so that it can be attached as a USB/hard-disk
device in VirtualBox for testing. The conversion uses `qemu-img convert`
from raw to VDI format. The resulting `.vdi` file is byte-equivalent to
the ISO but in a container format that VirtualBox recognises as a hard disk.

## CD-ROM Partition Scanning

> r[iso.cdrom-partscan+3]
> When the ISO is booted as optical media (e.g. `/dev/sr0` in a VM), the
> Linux kernel does not parse the GPT appended partitions because the CD-ROM
> block device driver exposes the device as a single block device with an
> ISO 9660 filesystem. As a result, partition device nodes (e.g. `sr0p3`,
> `sr0p4`) are never created and `/dev/disk/by-partuuid/` symlinks for the
> appended BESIMAGES and BESCONF partitions do not appear.
>
> The installer must handle this transparently. When the well-known
> PARTUUIDs are not present in `/dev/disk/by-partuuid/`, the installer
> must:
>
> 1. Identify the boot device by running `blkid -t LABEL=BES_INSTALLER`
>    (works even with `toram` where `/run/live/medium` is backed by
>    tmpfs), falling back to well-known CD-ROM paths (`/dev/sr0`,
>    `/dev/cdrom`).
> 2. Run `losetup --find --show --partscan --read-only <device>` to create
>    a loop device with partition scanning enabled.
> 3. Run `partprobe` on the loop device and wait for udev to settle.
> 4. After this, the kernel creates partition device nodes on the loop
>    device (e.g. `loop0p3`, `loop0p4`) and udev populates
>    `/dev/disk/by-partuuid/` with the well-known PARTUUIDs.
>
> This must happen early in the installer's startup, before any attempt to
> mount BESCONF or open the images partition. The installer must detach the
> loop device on exit. On USB boot, the PARTUUIDs are already visible and
> this step is a no-op.

## Integrity Verification

> r[iso.verity.layout+3]
> All verity-protected blobs (the squashfs rootfs and the images partition)
> must use a self-describing layout: `[data | verity hash tree | hash_size]`,
> where `hash_size` is a little-endian unsigned 64-bit integer (8 bytes) at
> the very end of the blob. The value of `hash_size` encodes the distance in
> bytes from the end of the data region to the start of the trailer, i.e.
> `hash_size = total_blob_size - 8 - data_size`. This distance includes the
> actual verity hash tree and any alignment padding that follows it.
>
> Every verity blob must be padded to a 4096-byte boundary so that tools
> which operate on sector-aligned data (`losetup`, `veritysetup`,
> `xorriso --append_partition`, block devices) see the trailer at exactly
> `total_size - 8`. Without padding, these tools silently round or truncate
> to a sector boundary and the trailer becomes unreachable. 4096 bytes is
> chosen because it satisfies the 512-byte requirement of `losetup`, the
> 2048-byte ISO 9660 sector size used by `xorriso`, and the 4096-byte page
> size commonly used by block device I/O. The build process must:
>
> 1. Record the data size before appending the hash tree.
> 2. Append the hash tree.
> 3. Compute the total size needed: round up `(current_size + 8)` to the
>    next 4096-byte boundary.
> 4. Pad with zero bytes to `total_needed - 8`.
> 5. Write the 8-byte trailer where `hash_size = total_needed - 8 - data_size`.
>
> The padding bytes between the hash tree and the trailer are harmless
> because `veritysetup` infers the hash tree extent from the data size and
> hash algorithm, ignoring any trailing content.
>
> At runtime, the consumer reads the last 8 bytes to recover `hash_size`,
> computes `hash_offset = total_size - 8 - hash_size`, and passes
> `--hash-offset=<hash_offset>` to `veritysetup open`. For block devices,
> `total_size` must be obtained via a method that returns the device size
> (e.g. seeking to the end), not `stat()` which returns 0 for block devices
> on Linux. No external metadata file is needed for offsets. The root hash is
> the only piece of information that must be stored externally (it is the
> trust anchor).

> r[iso.verity.squashfs+3]
> The live rootfs squashfs (`/live/filesystem.squashfs`) must be protected by
> dm-verity using the layout described in `r[iso.verity.layout+3]`. At build
> time:
>
> 1. Run `mksquashfs` to produce the squashfs.
> 2. Run `veritysetup format` on the squashfs to produce a hash tree file
>    and a root hash.
> 3. Append the hash tree to the squashfs file.
> 4. Pad the blob to 4096-byte alignment and write the trailer as described
>    in `r[iso.verity.layout+3]`.
> 5. Embed the root hash in the GRUB kernel command line as
>    `live.verity.roothash=<hex>`.
>
> The resulting file at `/live/filesystem.squashfs` inside the ISO contains
> `[squashfs | hash tree | padding | hash_size_le64]` as a single
> sector-aligned blob.
>
> The initramfs must include `veritysetup` and its runtime dependencies so
> that dm-verity can be set up during early boot. At boot, an initramfs
> premount script must:
>
> 1. Read the `live.verity.roothash=` parameter from `/proc/cmdline`.
> 2. Read the last 8 bytes of the squashfs file to recover the hash tree
>    size, and compute the hash offset.
> 3. Set up a loop device on the squashfs file.
> 4. Run `veritysetup open` with the loop device as both data and hash
>    device, passing `--hash-offset`.
> 5. Mount the resulting `/dev/mapper/live-verity` as the live root instead
>    of mounting the squashfs file directly.
>
> If the `live.verity.roothash=` parameter is absent from the kernel command
> line, the hook must be skipped and boot must proceed without verification
> (graceful fallback for development builds).

> r[iso.images-partition+3]
> The ISO must include a read-only squashfs partition appended via
> `xorriso --append_partition`. This squashfs must contain the raw
> (uncompressed) partition images (`efi.img`, `xboot.img`, `root.img`) and
> the `partitions.json` manifest. The squashfs must use zstd compression so
> that the kernel decompresses data transparently on read. The partition
> must have a well-known GPT PARTUUID of `ac9457d6-7d97-56bc-b6a6-d1bb7a00a45b`
> so that the installer can locate it via `/dev/disk/by-partuuid/` without
> depending on a specific partition number. After `xorriso` produces the
> ISO, the build script must use `sfdisk --part-uuid` to stamp this PARTUUID
> onto the images partition. On CD-ROM boot, the partition scanning service
> described in `r[iso.cdrom-partscan+3]` ensures these PARTUUIDs become
> available.

> r[iso.verity.images+4]
> The images squashfs partition must be protected by dm-verity using the
> layout described in `r[iso.verity.layout+3]`. At build time:
>
> 1. Run `mksquashfs` to produce the images squashfs.
> 2. Record the squashfs data size.
> 3. Run `veritysetup format` on the squashfs to produce a hash tree file
>    and a root hash.
> 4. Append the hash tree to the squashfs file.
> 5. Pad the blob to 4096-byte alignment and write the trailer as described
>    in `r[iso.verity.layout+3]`.
> 6. Append the combined blob as a GPT partition via xorriso.
> 7. Store the root hash in the GRUB kernel command line as
>    `images.verity.roothash=<hex>`.
>
> At runtime, the installer must find the images partition by its well-known
> PARTUUID (`ac9457d6-7d97-56bc-b6a6-d1bb7a00a45b`) via
> `/dev/disk/by-partuuid/`, read the last 8 bytes to recover the hash tree
> size and compute the hash offset, then run `veritysetup open` with the
> root hash and `--hash-offset`, mount the resulting dm-verity device as
> squashfs, and read partition images from it.

> r[iso.verity.failure]
> If dm-verity verification fails, the system must not silently use corrupted
> data. For the squashfs rootfs, the kernel returns I/O errors on corrupted
> blocks, causing boot to fail visibly. For the images partition, the
> installer must detect the `veritysetup open` failure or subsequent I/O
> errors and display an error screen. The error message depends on when the
> failure is detected:
>
> - **During the upfront integrity check** (before any writes): the error
>   must state that the installation media is corrupted, that the target disk
>   has **not** been written to, and that the only recourse is to write a new
>   copy of the installation medium.
>
> - **During the partition write phase**: the error must state that the
>   installation media is corrupted, that the target disk has been partially
>   written and cannot be used, and that the only recourse is to write a new
>   copy of the installation medium.

> r[iso.verity.check+5]
> On boot, after the images partition is opened via dm-verity, the installer
> must perform a full sequential read of every partition image file into
> `/dev/null` using `splice(2)` before beginning the installation. This
> forces dm-verity to verify every block of the images partition up front,
> catching corruption before any data is written to the target disk.
>
> In interactive (TUI) mode, the integrity check must run in the background
> while the welcome screen is displayed, starting automatically when the
> welcome screen is first shown. A progress bar labelled
> "Verifying installation media..." must be rendered at the bottom of the
> welcome screen. The user must not be allowed to advance past the welcome
> screen until the integrity check completes successfully. If the check
> fails, the installer must display the pre-write corruption error described
> in `r[iso.verity.failure]`. The `n` (network check) and `q` (reboot)
> keybinds remain available during the check.
>
> In automatic mode, the integrity check runs sequentially before the
> installation begins, with progress printed to stderr.
>
> If verity is not active (e.g. development builds using the fallback
> manifest path), the integrity check is skipped and the welcome screen
> does not block advancement.
>
> If any read returns an I/O error, the installer must display the pre-write
> corruption error described in `r[iso.verity.failure]`.

