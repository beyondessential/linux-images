# Raspberry Pi 5 image variant

## Goal

Add a `pi` image variant alongside `metal` and `cloud`, building bootable
Raspberry Pi 5 images from the same debootstrap-based pipeline. Boot stack
swaps to Pi firmware + `linux-raspi` + `flash-kernel`; everything post-boot
(LUKS, BTRFS subvolumes, cloud-init, snapper, tailscale, firewall, growth
service) is inherited from `metal`.

## Scope

- Pi 5 only (no Pi 4 / Pi 3).
- New variant `pi`. Always arm64. Always LUKS (like metal). No GRUB. No UEFI.
- No installer changes (Pi flow is flash-then-boot, not interactive install).
- No TPM logic (Pi has no native TPM; LUKS uses the existing empty-passphrase
  flow).

## Boot model

Pi 5 EEPROM boots directly. The bootloader reads `config.txt` from the first
FAT partition; `config.txt` references kernel + initramfs + DTB; kernel
command line lives in `cmdline.txt`. No GRUB, no U-Boot.

`flash-kernel` is responsible for populating `/boot/firmware` on kernel
package updates and at first build.

## Partition layout

Reuse the existing 3-partition GPT layout, with role of partition 1 changed
for the `pi` variant:

| # | Variant=metal/cloud           | Variant=pi                                        |
|---|-------------------------------|---------------------------------------------------|
| 1 | FAT, label `efi`, 512M, EFI   | FAT, label `firmware`, ~1G, mounted /boot/firmware |
| 2 | ext4, label `xboot`, 1G, /boot| ext4, label `xboot`, 1G, /boot                    |
| 3 | btrfs (LUKS for metal), /     | btrfs+LUKS, /                                     |

Pi firmware reads p1 (FAT). Kernel + initramfs + DTBs + overlays + Pi
bootloader config live there. `xboot` (p2) keeps its existing role for
`/boot` (kernel sources before flash-kernel copies into `/boot/firmware`,
plus dracut's preferred location for old initramfs).

## Files & changes

### `image/build.sh`
- Accept `VARIANT=pi`. Force `ARCH=arm64` when pi.
- Branch on variant for partition 1:
  - pi: size ≈ 1G, partition label `firmware`, partition type
    `0FC63DAF-8483-4772-8E79-3D69D8477DE4` (Linux filesystem) or keep ESP
    type for compatibility — TBD.
  - others: existing 512M EFI partition.
- Mount p1 at `/boot/firmware` instead of `/boot/efi` for pi.
- `GRUB_TARGET` becomes irrelevant for pi (pass empty / skip).
- LUKS handling unchanged from `metal`.

### `image/packages.sh`
- Split into a common base list and per-variant lists:
  - common: btrfs-progs, cryptsetup, snapper, gdisk, mtools, cloud-guest-utils,
    parted, netplan.io, openssh-server, curl, wget, ufw, cloud-init, chrony,
    systemd-resolved, rsync, cron, sudo, gnupg, nvme-cli, busybox, rng-tools5,
    jq, console-setup, kbd, neovim, nano, less, htop, iputils-ping,
    dracut-core.
  - metal/cloud: `linux-generic`, `grub-efi`, `tpm2-tools`.
  - pi: `linux-raspi`, `linux-firmware-raspi`, `flash-kernel`.

### `image/configure.sh`
- Branch boot setup:
  - non-pi: existing GRUB path.
  - pi:
    - Set `/etc/flash-kernel/machine` to `Raspberry Pi 5 Model B`.
    - Skip GRUB defaults setup, `update-grub`, `grub-install`.
    - Write `/boot/firmware/config.txt` (from `image/files/pi/config.txt`).
    - Write `/boot/firmware/cmdline.txt` with kernel cmdline (root=, rootflags,
      console settings, plus dracut LUKS options inherited via crypttab).
    - Run `flash-kernel --force` after `dracut --force` so kernel + initrd
      land in `/boot/firmware`.
- linux-firmware install: gate the existing `linux-firmware` install so it
  applies only to non-pi metal; pi uses `linux-firmware-raspi` (already in
  packages list).
- fstab branch for pi:
  - `/boot/firmware` instead of `/boot/efi` for p1, vfat, umask=0077.
  - p2 unchanged (xboot → /boot).
  - p3 unchanged (LUKS+btrfs → /).
- crypttab unchanged from metal.

### `image/files/pi/config.txt`
New file. Minimal Pi 5 boot config — `arm_64bit=1`, `kernel=`, `initramfs`,
appropriate device-tree directive for Pi 5.

### `image/files/dracut/`
No new files needed; existing portable-image / hardware-driver configs work
for arm64. Optionally drop TPM modules from pi-built initramfs later (not in
scope for v1).

### `tests/test-image-structure.sh`
- Recognise `variant=pi` (currently rejects anything not metal/cloud).
- For pi:
  - p1 label = `firmware`, mounted as vfat.
  - `/boot/firmware/config.txt` and `/boot/firmware/cmdline.txt` present.
  - kernel image and initrd present in `/boot/firmware`.
  - `linux-raspi` and `flash-kernel` packages installed.
  - GRUB packages NOT installed; `/boot/grub` empty/absent.
  - `/etc/bes/image-variant` contains `pi`.
- LUKS / BTRFS subvolume / snapper / firewall / tailscale checks remain
  shared (already correct for metal-style variants).

### `justfile`
- Add `pi` to `_validate-variant`.
- Force `arch=arm64` when `variant=pi` (or error if mismatched).
- Update `_default` help text and any variant enumerations
  (`build-all-variants`, `build-all`).

### `docs/spec/disk-images.md`
- New section / variant entries describing the pi variant boot stack and
  partition role differences.
- New tracey tags: `image.variant.pi`, `image.boot.pi-firmware`,
  `image.boot.pi-kernel`, `image.boot.pi-flash-kernel` etc. as needed for
  spec coverage.

### `docs/GUIDE-IMAGES.md`
- Add a row to the variant table (Raspberry Pi 5).
- Note Pi-specific flashing instructions (rpi-imager / dd to SD or NVMe).

## Open questions / risks

- **flash-kernel in chroot**: cross-build chroot needs `FK_MACHINE` set
  explicitly so flash-kernel doesn't try to detect from `/proc/device-tree`
  on the build host. May also need `FK_FORCE=yes` and bypassing the
  `/proc/cpuinfo` machine detection.
- **linux-firmware-raspi vs linux-firmware**: linux-firmware-raspi is much
  smaller; if any add-on hardware needs firmware not in the Pi-specific
  package, layer linux-firmware on top later. Out of scope for v1.
- **NVMe HAT**: Pi 5 with NVMe HAT is the expected target; dracut already
  pulls nvme-cli and the kernel module. Test on real hardware.
- **linux-raspi cadence**: separate kernel series from linux-generic, with
  its own update timing. Mention in release notes.
- **Hostname/networking on first boot**: cloud-init handles this;
  the existing `bes-tailscale-firstboot-auth` flow should run unchanged.
- **Pi 5 partition GUIDs**: confirm Pi firmware accepts GPT with our chosen
  type GUIDs. If not, use ESP type for p1 and just relabel.

## Out of scope (deliberate)

- Installer integration (Pi is flashed, not installed via TUI).
- Older Pi models.
- Wifi/Bluetooth provisioning beyond what the existing image stack already
  carries.
- Per-Pi-model auto-detection (target is fixed: Pi 5).
- Removing TPM tooling from the pi initramfs (cheap dead weight; defer).

## Commit plan (incremental)

1. `plan: raspberry-pi-variant`
2. `image: accept pi variant in build.sh and justfile (skeleton)`
3. `image: split packages list into common + per-variant`
4. `image: add Pi 5 firmware boot config files`
5. `image: build pi variant with firmware partition layout`
6. `image: configure pi boot stack via flash-kernel`
7. `test: add pi variant assertions to test-image-structure`
8. `justfile: pi variant build recipes and validation`
9. `spec: document pi image variant`
10. `unplan`

A bookmark `pi5-variant` is created on the tip after `unplan`.
