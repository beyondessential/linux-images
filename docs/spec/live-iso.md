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

r[iso.contents+3]
The ISO must contain a TUI installer binary and a `partitions.json` manifest
describing the partition layout. There is one set of partition images per
architecture, not per variant. The partition images (`efi.img`, `xboot.img`,
`root.img`) are stored as raw (uncompressed) files inside a dedicated
squashfs with transparent zstd compression (see `r[iso.images-partition]`).
The installer reconstructs the GPT and writes each partition individually,
setting up LUKS on the root partition when encryption is enabled.

r[iso.boot.uefi]
The ISO must be UEFI-bootable via an El Torito EFI boot catalog. The EFI
boot image is a FAT32 filesystem image containing a GRUB EFI binary at
the default removable media path (`EFI/BOOT/BOOTX64.EFI` for amd64,
`EFI/BOOT/BOOTAA64.EFI` for arm64).

r[iso.boot.autostart+3]
On boot, the live environment must automatically launch the TUI installer on
tty2. A separate oneshot service must switch the active VT to tty2 before
the installer starts, so the user sees the installer immediately. The `kbd`
package must be installed in the live rootfs to provide `/usr/bin/chvt`.
The `systemd-sysv` package must be installed to provide `/sbin/reboot`
so that the installer's reboot action works.

> r[iso.config-partition]
> The ISO must include an appended FAT32 partition (GPT type `Microsoft basic
> data`) created via `xorriso --append_partition`. This partition is embedded
> in the ISO file and becomes a real writable GPT partition after the ISO is
> written to USB via `dd`. Its filesystem label must be `BESCONF`.
>
> When booted from USB, this partition is writable and is the intended location
> for users to place a `bes-install.toml` configuration file before booting.
> When booted as optical media in a VM, the partition is still readable.

r[iso.per-arch]
Separate ISO images must be produced per architecture (amd64, arm64). Each
ISO contains only the images and installer binary for its architecture.

r[iso.usb]
The ISO must be writable to USB media using `dd` and must boot correctly on
UEFI systems from that media.

## Integrity Verification

> r[iso.verity.layout]
> All verity-protected blobs (the squashfs rootfs and the images partition)
> must use a self-describing layout: `[data | verity hash tree | hash_size]`,
> where `hash_size` is the size of the verity hash tree in bytes, stored as a
> little-endian unsigned 64-bit integer (8 bytes) at the very end of the
> blob. At runtime, the consumer reads the last 8 bytes to recover
> `hash_size`, computes `hash_offset = total_size - 8 - hash_size`, and
> passes `--hash-offset=<hash_offset>` to `veritysetup open`. No external
> metadata file is needed for offsets. The root hash is the only piece of
> information that must be stored externally (it is the trust anchor).

> r[iso.verity.squashfs+2]
> The live rootfs squashfs (`/live/filesystem.squashfs`) must be protected by
> dm-verity using the layout described in `r[iso.verity.layout]`. At build
> time:
>
> 1. Run `mksquashfs` to produce the squashfs.
> 2. Run `veritysetup format` on the squashfs to produce a hash tree file
>    and a root hash.
> 3. Append the hash tree to the squashfs file.
> 4. Append the hash tree size as a little-endian u64 (8 bytes).
> 5. Embed the root hash in the GRUB kernel command line as
>    `live.verity.roothash=<hex>`.
>
> The resulting file at `/live/filesystem.squashfs` inside the ISO contains
> `[squashfs | hash tree | hash_size_le64]` as a single blob.
>
> At boot, a custom initramfs premount script must:
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

> r[iso.verity.initramfs-hook+2]
> The live rootfs must include an initramfs hook at
> `/usr/share/initramfs-tools/hooks/verity` that copies `veritysetup` and its
> runtime dependencies (shared libraries, `libcryptsetup`, `libdevmapper`)
> into the initramfs. A premount script at
> `/usr/share/initramfs-tools/scripts/live-premount/verity` must implement
> the dm-verity setup described in `r[iso.verity.squashfs]`, including the
> trailer read to recover the hash offset.

> r[iso.images-partition]
> The ISO must include a read-only squashfs partition appended as GPT
> partition 4 via `xorriso --append_partition`. This squashfs must contain
> the raw (uncompressed) partition images (`efi.img`, `xboot.img`,
> `root.img`) and the `partitions.json` manifest. The squashfs must use zstd
> compression so that the kernel decompresses data transparently on read.
> The filesystem label must be `BESIMAGES`.

> r[iso.verity.images+2]
> The images squashfs partition must be protected by dm-verity using the
> layout described in `r[iso.verity.layout]`. At build time:
>
> 1. Run `mksquashfs` to produce the images squashfs.
> 2. Run `veritysetup format` on the squashfs to produce a hash tree file
>    and a root hash.
> 3. Append the hash tree to the squashfs file.
> 4. Append the hash tree size as a little-endian u64 (8 bytes).
> 5. Append the combined blob as GPT partition 4 via xorriso.
> 6. Store the root hash in the GRUB kernel command line as
>    `images.verity.roothash=<hex>`.
>
> At runtime, the installer must find the images partition (GPT partition 4
> of the boot device, or by filesystem label `BESIMAGES`), read the last 8
> bytes to recover the hash tree size and compute the hash offset, then run
> `veritysetup open` with the root hash and `--hash-offset`, mount the
> resulting dm-verity device as squashfs, and read partition images from it.

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

> r[iso.verity.check+2]
> On boot, after the images partition is opened via dm-verity, the installer
> must perform a full sequential read of every partition image file into
> `/dev/null` using `splice(2)` before beginning the installation. This
> forces dm-verity to verify every block of the images partition up front,
> catching corruption before any data is written to the target disk. The
> installer must display progress during this check. If any read returns an
> I/O error, the installer must display the pre-write corruption error
> described in `r[iso.verity.failure]`.

r[iso.verity.build-deps]
The ISO build script must have `cryptsetup` (which provides `veritysetup`)
available as a build-time dependency for computing dm-verity hash trees.
