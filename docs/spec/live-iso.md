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

r[iso.minimal+2]
The live environment must include a kernel, an initramfs, and enough
userspace to run the TUI installer (block device utilities, zstd, and
cryptsetup for LUKS operations). The default debootstrap variant provides
the base; only packages not included in it need to be installed explicitly.

r[iso.verify-paths]
The ISO structure test must invoke the installer binary with `--check-paths`
against the mounted squashfs rootfs to verify that every hardcoded external
binary path resolves to an existing file. This catches path mismatches
between the installer and the packages installed in the live environment.

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
