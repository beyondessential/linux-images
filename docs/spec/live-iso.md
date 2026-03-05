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

r[iso.base]
The live rootfs must be built with `debootstrap` (not `live-build`) for
consistency with the disk image builder. The rootfs is packaged as a
squashfs and placed inside the ISO.

r[iso.live-boot]
The live environment must include the `live-boot` and `live-boot-initramfs-tools`
packages so that the kernel can locate and mount the squashfs root via the
`boot=live` parameter. The squashfs must be placed at `/live/filesystem.squashfs`
inside the ISO, which is the default path `live-boot` searches.

r[iso.minimal]
The live environment must be minimal: a kernel, an initramfs, and just enough
userspace to run the TUI installer (block device utilities, zstd, and
cryptsetup for LUKS operations).

r[iso.network-tools]
The live environment must include `curl` (for HTTPS connectivity checks and
GitHub SSH key lookups) and `tailscale` (for running `tailscale netcheck`
diagnostics during installation). These are used by the interactive TUI
screens for network checks and are not required for offline installation.

r[iso.offline]
The live ISO must be fully functional without network connectivity. No
packages or data are downloaded during the installation process.

r[iso.contents+2]
The ISO must contain compressed partition images extracted from the cloud disk
image, a `partitions.json` manifest describing the partition layout, and the
TUI installer binary. There is one set of partition images per architecture,
not per variant. The partition images are `efi.img.zst`, `xboot.img.zst`, and
`root.img.zst`, each with a `.size` sidecar containing the uncompressed byte
count. The installer reconstructs the GPT and writes each partition
individually, setting up LUKS on the root partition when encryption is enabled.

r[iso.boot.uefi]
The ISO must be UEFI-bootable via an El Torito EFI boot catalog. The EFI
boot image is a FAT32 filesystem image containing a GRUB EFI binary at
the default removable media path (`EFI/BOOT/BOOTX64.EFI` for amd64,
`EFI/BOOT/BOOTAA64.EFI` for arm64).

r[iso.boot.autostart]
On boot, the live environment must automatically launch the TUI installer on
the primary console.

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
