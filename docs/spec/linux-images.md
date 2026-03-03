# BES Linux Images

This specification defines the requirements for building BES's custom Ubuntu
Server disk images and the live USB installer used to deploy them to physical
machines.

The system has two major components: a **build pipeline** that produces minimal
disk images directly (without booting an installer), and a **live ISO installer**
that writes those images to physical hardware, optionally driven by a TOML
configuration file.

# Disk Images

## Partition Layout

r[image.partition.table]
The disk image must use a GPT partition table.

r[image.partition.count]
The disk image must contain exactly three partitions: EFI, extended boot, and
root. There is no swap partition.

r[image.partition.efi]
The first partition must be an EFI System Partition (type UUID
`C12A7328-F81F-11D2-BA4B-00A0C93EC93B`), formatted as FAT32, labeled `efi`,
and sized 512 MiB.

r[image.partition.xboot]
The second partition must be a Linux extended boot partition (type UUID
`BC13C2FF-59E6-4262-A352-B275FD6F7172`), formatted as ext4, labeled `xboot`,
and sized 1 GiB.

> r[image.partition.root]
> The third partition must be a Linux root partition, labeled `root`, using all
> remaining disk space. The type UUID must be architecture-specific:
>
> - amd64: `4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709`
> - arm64: `B921B045-1DF0-41C3-AF44-4C6F280D3FAE`

## BTRFS Configuration

r[image.btrfs.format]
The root partition (or the LUKS volume on top of it, for the metal variant)
must be formatted as BTRFS with the label `ROOT`, the xxhash checksum
algorithm, and the `block-group-tree` and `squota` features enabled.

r[image.btrfs.subvolumes]
The BTRFS filesystem must contain two subvolumes: `@` mounted at `/`, and
`@postgres` mounted at `/var/lib/postgresql`.

r[image.btrfs.compression]
All BTRFS mounts must use transparent zstd compression at level 6 via the
`compress=zstd:6` mount option.

r[image.btrfs.quotas]
BTRFS simple quotas must be enabled on the filesystem.

## Variants

r[image.variant.types]
Two image variants must be supported: `metal` and `cloud`.

r[image.variant.metal]
The `metal` variant encrypts the root partition with LUKS2 and includes
TPM auto-enrollment support. It is intended for bare-metal and on-premise
virtualisation.

r[image.variant.cloud]
The `cloud` variant does not encrypt the root partition. It is intended for
cloud environments where encryption at rest is provided by the infrastructure.

r[image.variant.persisted]
The active variant name must be written to `/etc/bes/image-variant` in the
installed system so that runtime scripts can branch on it.

## Base System

r[image.base.debootstrap]
The base system must be bootstrapped using debootstrap from the Ubuntu 24.04
(Noble Numbat) repositories into the `@` subvolume.

r[image.base.minimal]
The debootstrap must use the `minbase` variant. Additional packages are
installed via apt after the initial bootstrap.

r[image.base.machine-id]
`/etc/machine-id` must be truncated to zero bytes so that systemd generates a
unique machine ID on each first boot.

r[image.base.resolv-conf]
`/etc/resolv.conf` must be a symlink to `/run/systemd/resolve/stub-resolv.conf`.

## Package Installation

r[image.packages.list]
Packages to install must be defined in a text file (`packages.txt`), one
package per line. Lines starting with `#` are comments and blank lines are
ignored.

r[image.packages.install]
All packages in the package list must be installed via apt inside the chroot
after the base system is bootstrapped.

## Bootloader

r[image.boot.dracut]
The initramfs must be generated using dracut, not initramfs-tools. Dracut must
be configured with `hostonly="yes"` and `hostonly_mode="sloppy"`.

r[image.boot.grub-install]
GRUB must be installed as the EFI bootloader with `--bootloader-id=ubuntu`.

r[image.boot.grub-timeout]
GRUB must be configured with `GRUB_TIMEOUT=5`,
`GRUB_TIMEOUT_STYLE=menu`, and `GRUB_RECORDFAIL_TIMEOUT=5`.

r[image.boot.grub-cmdline]
The GRUB kernel command line must include `noresume`.

## Firewall

r[image.firewall.policy]
UFW must be configured with default deny incoming, allow outgoing, and allow
forwarding policies.

r[image.firewall.ssh]
SSH (port 22/tcp) must be allowed from all sources by default. This rule is
removed when Tailscale connects successfully.

r[image.firewall.http]
HTTP (port 80/tcp), HTTPS (port 443/tcp), and HTTP/3 (port 443/udp) must be
allowed.

r[image.firewall.enabled]
UFW must be force-enabled during image configuration.

## Tailscale

r[image.tailscale.repo]
Tailscale must be installed from the official Tailscale apt repository. The
repository signing key must be pre-installed at
`/usr/share/keyrings/tailscale-archive-keyring.gpg`.

r[image.tailscale.pinned]
An apt pin at priority 900 must be configured for the Tailscale repository.

r[image.tailscale.service-enabled]
The `tailscaled` systemd service must be enabled but Tailscale must not be
joined to any tailnet — that happens either via `ts-up` or via the installer's
first-boot configuration.

r[image.tailscale.ts-up]
A helper script must be installed at `/usr/local/bin/ts-up` that interactively
prompts for a Tailscale auth key, connects to the tailnet with `--ssh`, and on
success restricts the SSH UFW rule to LAN ranges (RFC 1918, ULA) and the
`tailscale0` interface.

r[image.tailscale.auto-update]
A weekly cron job must be present to run `apt install -y tailscale`.

## Snapper

r[image.snapper.root]
Snapper must be configured for the root subvolume (`/`) with timeline
snapshots enabled and retention of 10 hourly, 7 daily, 4 weekly, and
12 monthly snapshots.

r[image.snapper.postgres]
Snapper must be configured for the PostgreSQL subvolume
(`/var/lib/postgresql`) with the same retention settings as the root config.

r[image.snapper.timers]
The `snapper-timeline.timer` and `snapper-cleanup.timer` systemd timers must
be enabled.

## Disk Growth

r[image.growth.service]
A systemd service `grow-root-filesystem.service` must run early at boot
(before user sessions, before LUKS re-encryption) to expand the root partition
and filesystem if additional disk space is available.

> r[image.growth.script]
> The growth script at `/usr/local/bin/grow-root-filesystem` must, in order:
>
> 1. Move the GPT secondary header to the end of the disk.
> 2. Run `growpart` on the root partition.
> 3. If the metal variant, run `cryptsetup resize root`.
> 4. Run `btrfs filesystem resize max /`.

## Credentials

r[image.credentials.ubuntu-user]
A `ubuntu` user must exist with the pre-set password `bes`. The password must
be marked expired so that console login forces an immediate password change.

r[image.credentials.root-disabled]
The `root` user must have its shell set to `/sbin/nologin`.

r[image.credentials.ssh-keys-only]
SSH password authentication must be disabled. Only key-based authentication
is permitted over SSH.

## Cloud-Init

r[image.cloud-init.enabled]
cloud-init must be installed and enabled, allowing cloud providers and NoCloud
data sources to inject configuration (networking, SSH keys) at first boot.

r[image.cloud-init.no-hostname-file]
cloud-init must be configured with `create_hostname_file: false` so that the
hostname is provided by DHCP rather than a static file.

## Encryption (Metal Variant)

### LUKS

r[image.luks.format]
The root partition must be formatted with LUKS2 using an empty passphrase in
key slot 0.

r[image.luks.keyfile]
An empty keyfile must be installed at `/etc/luks/empty-keyfile` with mode 000.
Dracut must be configured to include this keyfile in the initramfs.

> r[image.luks.crypttab]
> `/etc/crypttab` must contain an entry mapping `/dev/disk/by-partlabel/root`
> to the name `root` with the keyfile `/etc/luks/empty-keyfile` and options:
>
> `force,luks,discard,headless=true,try-empty-password=true`
>
> The `force` option is required because dracut otherwise skips entries that
> have a keyfile configured.

r[image.luks.reencrypt]
A first-boot systemd service `luks-reencrypt.service` must re-encrypt the
LUKS volume with a new randomly-generated master key, so that each
installation has unique key material. The service must disable itself after
running.

### TPM Auto-Enrollment

r[image.tpm.service]
A systemd service `setup-tpm-unlock.service` must be installed, conditioned on
the presence of `/dev/tpm0` and `/dev/tpmrm0` and the absence of
`/etc/luks/tpm-enrolled`.

r[image.tpm.enrollment]
The TPM enrollment script must use `systemd-cryptenroll` to bind the LUKS
volume to TPM2 PCR 7 using the empty keyfile for unlock, then remove the
password key slot, update `/etc/crypttab` to use `tpm2-device=auto`, mark
enrollment complete, and regenerate the initramfs.

r[image.tpm.disableable]
TPM auto-enrollment must be disableable. When disabled, the
`setup-tpm-unlock.service` unit must not be enabled in the installed system.
The LUKS volume remains unlockable via the empty passphrase.

## fstab

> r[image.fstab.metal]
> The metal variant `/etc/fstab` must contain:
>
> | Device | Mount | FS | Options |
> |---|---|---|---|
> | `/dev/mapper/root` | `/` | btrfs | `subvol=@,compress=zstd:6` |
> | `/dev/mapper/root` | `/var/lib/postgresql` | btrfs | `subvol=@postgres,compress=zstd:6` |
> | `/dev/disk/by-partlabel/xboot` | `/boot` | ext4 | `defaults` |
> | `/dev/disk/by-partlabel/efi` | `/boot/efi` | vfat | `umask=0077` |

> r[image.fstab.cloud]
> The cloud variant `/etc/fstab` must contain:
>
> | Device | Mount | FS | Options |
> |---|---|---|---|
> | `/dev/disk/by-partlabel/root` | `/` | btrfs | `subvol=@,compress=zstd:6` |
> | `/dev/disk/by-partlabel/root` | `/var/lib/postgresql` | btrfs | `subvol=@postgres,compress=zstd:6` |
> | `/dev/disk/by-partlabel/xboot` | `/boot` | ext4 | `defaults` |
> | `/dev/disk/by-partlabel/efi` | `/boot/efi` | vfat | `umask=0077` |

## Post-Processing

r[image.postprocess.defrag]
The `@` subvolume must be defragmented with zstd compression at level 15
before the image is finalized.

r[image.postprocess.dedupe]
The `@` subvolume must be block-level deduplicated using duperemove before the
image is finalized.

r[image.postprocess.cleanup]
Installer artifacts, cloud-init installer network configs
(`/etc/cloud/cloud.cfg.d/90-installer-network.cfg`), and the unminimize
prompt (`/etc/update-motd.d/60-unminimize`) must be removed.

## Output

r[image.output.raw]
A raw disk image file (`.raw`) must be produced.

r[image.output.vmdk]
A VMDK image with `streamOptimized` subformat must be produced from the raw
image using `qemu-img convert`.

r[image.output.qcow2]
A qcow2 image with zstd compression must be produced from the raw image using
`qemu-img convert`.

r[image.output.compress]
The raw image must be compressed with zstd at level 6, producing a `.raw.zst`
file. The uncompressed `.raw` is removed.

r[image.output.checksum]
SHA256 checksums of all output files must be written to a `SHA256SUMS` file
in the output directory.

# Build Process

r[build.direct]
Images must be built using debootstrap and chroot on a loopback-mounted raw
file. The build must not require QEMU, an Ubuntu ISO, or an autoinstall
process.

r[build.architectures]
Both amd64 and arm64 images must be producible.

r[build.cross-arch]
Building images for a foreign architecture (e.g. arm64 on an amd64 host)
must be supported via qemu-user-static and binfmt_misc for the chroot steps.

r[build.privileged]
The image build requires root privileges for loopback device setup,
partitioning, filesystem creation, and chroot.

r[build.container-postprocess]
Post-processing (defrag, dedupe) must run inside a container to isolate
privileged loopback and device-mapper operations from the host.

r[build.idempotent]
Running a clean build twice with the same inputs must produce
bit-for-bit identical images (excluding timestamps embedded by tools
outside our control, such as filesystem UUIDs).

# Installer

## Configuration File

r[installer.config.location]
The installer must look for a TOML configuration file named
`bes-install.toml` at the root of the EFI partition on the boot media.

> r[installer.config.schema]
> The configuration file has the following schema:
>
> ```toml
> # Run fully automatically without prompts.
> # Requires at minimum: variant and disk.
> auto = true
>
> # Image variant: "metal" or "cloud"
> variant = "metal"
>
> # Target disk: a device path or a strategy.
> # Strategies: "largest-ssd", "largest", "smallest"
> disk = "largest-ssd"
>
> # Disable TPM auto-enrollment (metal variant only).
> disable-tpm = false
>
> [firstboot]
> hostname = "server-01"
> tailscale-authkey = "tskey-auth-xxxxx"
> ssh-authorized-keys = [
>   "ssh-ed25519 AAAA... admin@example.com",
> ]
> ```
>
> All fields are optional. The `[firstboot]` table and all its fields are
> optional.

## Operating Modes

r[installer.mode.interactive]
When no configuration file is found, the installer must launch a fully
interactive TUI with sensible defaults (variant `metal`, disk strategy
`largest-ssd`).

r[installer.mode.prefilled]
When a configuration file is present but `auto` is false or absent, the
installer must launch the TUI with values from the file pre-filled as
defaults. The user can override any value.

> r[installer.mode.auto]
> When `auto = true` and all required fields (`variant`, `disk`) are present,
> the installer must proceed without any interactive prompts. It must:
>
> 1. Log its configuration to the console.
> 2. Display progress during image writing.
> 3. Apply first-boot configuration.
> 4. Reboot automatically on success.
> 5. Print an error and exit with a non-zero status on failure.

r[installer.mode.auto-incomplete]
When `auto = true` but required fields (`variant` or `disk`) are missing, the
installer must print an error describing the missing fields and fall back to
interactive mode.

## TUI

r[installer.tui.rust]
The TUI must be a Rust binary using the ratatui library, compiled as a
statically-linked executable.

r[installer.tui.disk-detection]
The TUI must detect available block devices and display their device path,
size, model name, and transport type (SSD, HDD, NVMe, USB, etc.).

r[installer.tui.variant-selection]
The TUI must present a choice between the `metal` and `cloud` variants with
a brief description of each.

r[installer.tui.tpm-toggle]
When the `metal` variant is selected, the TUI must offer a toggle to disable
TPM auto-enrollment.

r[installer.tui.confirmation]
Before writing, the TUI must show a summary screen listing: target disk
(path, model, size), chosen variant, TPM enrollment status, and any
first-boot configuration. The summary must clearly state that all data on
the target disk will be destroyed. The user must type an explicit confirmation
(not just press Enter).

r[installer.tui.progress]
During image writing, the TUI must display a progress bar showing bytes
written and estimated time remaining.

## Image Writing

r[installer.write.source]
Compressed disk images (`.raw.zst`) must be stored on the ISO filesystem. The
installer must select the correct image for the running CPU architecture and
chosen variant.

r[installer.write.decompress-stream]
The installer must stream-decompress the zstd image directly to the target
block device, avoiding the need to hold the uncompressed image in memory or
on a temporary filesystem.

r[installer.write.verify]
After writing, the installer should read back the partition table from the
target disk and verify that the expected partitions (efi, xboot, root) are
present with correct labels.

## First-Boot Configuration

r[installer.firstboot.mount]
After writing the image, the installer must mount the target disk's root
BTRFS partition (subvol `@`) to apply first-boot configuration. For the metal
variant, it must unlock the LUKS volume using the empty keyfile first.

r[installer.firstboot.hostname]
If `hostname` is set, the installer must write it to `/etc/hostname` on the
installed system.

r[installer.firstboot.tailscale-authkey]
If `tailscale-authkey` is set, the installer must write the key to
`/etc/bes/tailscale-authkey` and enable a first-boot systemd service that
runs `tailscale up --auth-key=<key> --ssh`, restricts SSH via UFW, and then
deletes the key file.

r[installer.firstboot.ssh-keys]
If `ssh-authorized-keys` is set, the installer must append each key to
`/home/ubuntu/.ssh/authorized_keys` with correct ownership and permissions
(directory 700, file 600, owned by `ubuntu`).

r[installer.firstboot.tpm-disable]
If `disable-tpm` is true, the installer must remove the
`setup-tpm-unlock.service` enable symlink from the installed system.

r[installer.firstboot.unmount]
After applying configuration, the installer must cleanly unmount all
filesystems and close any LUKS volumes before prompting for reboot.

# Live ISO

r[iso.base]
The live ISO must use a Debian Live environment as its base, built with the
`live-build` toolchain.

r[iso.minimal]
The live environment must be minimal: a kernel, an initramfs, and just enough
userspace to run the TUI installer (block device utilities, zstd, and
cryptsetup for LUKS operations).

r[iso.offline]
The live ISO must be fully functional without network connectivity. No
packages or data are downloaded during the installation process.

r[iso.contents]
The ISO must contain the compressed disk images (`.raw.zst`) for all variants
of the ISO's architecture, and the TUI installer binary.

r[iso.boot.uefi]
The ISO must be UEFI-bootable.

r[iso.boot.autostart]
On boot, the live environment must automatically launch the TUI installer on
the primary console.

r[iso.efi-writable]
After the ISO is written to a USB drive, the EFI System Partition must remain
writable (FAT32) so that users can place a `bes-install.toml` file on it
before booting.

r[iso.per-arch]
Separate ISO images must be produced per architecture (amd64, arm64). Each
ISO contains only the images and installer binary for its architecture.

r[iso.usb]
The ISO must be writable to USB media using `dd` and must boot correctly on
UEFI systems from that media.

# Testing

## Static Analysis

r[test.static.shellcheck]
All shell scripts in the repository must pass shellcheck with no errors.

r[test.static.cargo-test]
The TUI installer project must pass `cargo test` with no failures. Unit tests
must cover configuration parsing, disk strategy selection, and progress
calculation.

## Image Structure Verification

r[test.structure.method]
After building an image, automated verification must loopback-mount it and
assert correctness without booting. This must run in CI without KVM.

r[test.structure.partitions]
Verification must check that the partition table contains the expected number
of partitions with the correct labels and type UUIDs.

r[test.structure.filesystems]
Verification must check filesystem types (FAT32, ext4, BTRFS) and labels.

r[test.structure.subvolumes]
Verification must check that BTRFS subvolumes `@` and `@postgres` exist and
that simple quotas are enabled.

> r[test.structure.files]
> Verification must check that the following files exist in the root subvolume:
>
> - `/etc/fstab`
> - `/etc/bes/image-variant`
> - `/usr/local/bin/ts-up`
> - `/usr/local/bin/grow-root-filesystem`
> - `/etc/systemd/system/grow-root-filesystem.service`

r[test.structure.services]
Verification must check that expected systemd services are enabled by
confirming the presence of the appropriate symlinks in `.wants/` directories.

r[test.structure.packages]
Verification must check that every package listed in `packages.txt` is
recorded as installed in the image's `/var/lib/dpkg/status`.

r[test.structure.fstab]
Verification must check that the fstab entries match the expected values for
the variant under test (per r[image.fstab.metal] or r[image.fstab.cloud]).

r[test.structure.variant-specific]
For the metal variant, verification must additionally confirm the existence
of `/etc/crypttab`, `/etc/luks/empty-keyfile`, and LUKS metadata on the root
partition.

## Boot Smoke Test

r[test.boot.method]
The image must be booted in QEMU with a cloud-init NoCloud datasource
attached as a second virtual disk. QEMU must be configured to prefer KVM
acceleration with a TCG (software emulation) fallback, so the test can
run on hosts where KVM is unavailable. KVM is strongly preferred for
acceptable speed.

> r[test.boot.checks]
> The cloud-init injected test script must verify:
>
> - systemd has reached `multi-user.target` with no failed units
> - All expected services are active: sshd, ufw, tailscaled,
>   snapper-timeline.timer, grow-root-filesystem.service
> - Filesystem mounts match `/etc/fstab`
> - BTRFS compression is active (check `/proc/mounts` for `compress=`)
> - For metal: the LUKS volume unlocked successfully
> - The `ubuntu` user can be resolved
> - `/etc/machine-id` is non-empty (was regenerated at boot)

r[test.boot.output]
The test script must write results to the serial console. Each check must
print `PASS: <description>` or `FAIL: <description>`. A final line of
`TEST_SUCCESS` or `TEST_FAILURE` must be printed as the overall result.

r[test.boot.timeout]
The test harness must enforce a timeout (default 5 minutes). If the marker
line is not seen within the timeout, the test is considered failed.

r[test.boot.poweroff]
After printing results, the test script must power off the VM so the
harness can exit cleanly.

## End-to-End Install Test

r[test.e2e.method]
The live ISO must be booted in QEMU with a blank virtual disk attached and a
`bes-install.toml` config injected into the ISO's EFI partition to drive a
fully automatic install.

r[test.e2e.reboot]
After the installer finishes and the VM reboots, the harness must boot from
the target disk (not the ISO) and run the boot smoke test checks defined
in r[test.boot.checks].

r[test.e2e.variants]
The end-to-end test must be run for both the `metal` and `cloud` variants.

# CI/CD

r[ci.matrix]
CI must build images for all combinations of architecture (amd64, arm64) and
variant (metal, cloud).

r[ci.test-structure]
Image structure verification (r[test.structure.method]) must run on every
build.

r[ci.test-boot]
Boot smoke tests (r[test.boot.method]) must run when KVM is usable on the
runner. KVM usability must be verified by actually testing access (e.g.
checking the device is writable), not merely by checking that the device
node exists.

r[ci.test-e2e]
End-to-end install tests (r[test.e2e.method]) must run when KVM is usable
on the runner, using the same usability check as r[ci.test-boot].

r[ci.tui-build]
The TUI installer must be compiled for both amd64 and arm64 in CI. For the
foreign architecture, cross-compilation must be used.

r[ci.iso-build]
Live ISOs must be built for each architecture, containing that architecture's
images and TUI binary.

> r[ci.release]
> On tag push or manual workflow dispatch, CI must create a GitHub release
> containing:
>
> - Per architecture: `.raw.zst`, `.vmdk`, `.qcow2` for each variant
> - Per architecture: one live installer `.iso`
> - A combined `SHA256SUMS` file covering all release assets