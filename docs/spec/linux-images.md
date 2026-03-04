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

> r[image.variant.types]
> Two image variants must be supported: `metal` and `cloud`.
>
> The `metal` variant encrypts the root partition with LUKS2 and includes
> TPM auto-enrollment support. It is intended for bare-metal and on-premise
> virtualisation.
>
> The `cloud` variant does not encrypt the root partition. It is intended for
> cloud environments where encryption at rest is provided by the infrastructure.
>
> The active variant name must be written to `/etc/bes/image-variant` in the
installed system so that runtime scripts can branch on it.

## Base System

r[image.base.debootstrap]
The base system must be debootstrapped into the `@` subvolume.
The debootstrap must create the minimal viable bootable system.

r[image.base.machine-id]
`/etc/machine-id` must be truncated to zero bytes so that systemd generates a
unique machine ID on each first boot.

r[image.base.resolver]
systemd-resolved must be enabled and configured as the system DNS resolver.

## Packages

r[image.packages.bes-tools]
The bes-tools APT repo must be configured and preferred.

r[image.packages.caddy]
Caddy version >=2.10.0 must be pre-installed.

r[image.packages.podman]
Podman version >=5.0.0 must be pre-installed.

r[image.packages.kopia]
Kopia version >=0.22.0 must be pre-installed.
The official Kopia apt repository must be configured and preferred.

r[image.packages.tailscale]
Tailscale version >=1.92.0 must be pre-installed.
The official Tailscale apt repository must be configured and preferred.

r[image.packages.bestool+2]
bestool version >=1.4.0 must be pre-installed.

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

r[image.cloud-init.no-network]
cloud-init must not have network configs in the image.

r[image.cloud-init.no-machineid]
The /etc/machine-id file must be blank in the image so it's unique per install.

## Hostname

r[image.hostname.metal-dhcp]
The metal image must ship with an empty `/etc/hostname` (zero bytes) so that
`systemd-hostnamed` accepts DHCP-provided transient hostnames. `/etc/hosts`
must contain only `localhost` entries (no `127.0.1.1` line).

r[image.hostname.cloud-default]
The cloud image must ship with `ubuntu` as the static hostname in
`/etc/hostname`. Cloud-init with `create_hostname_file: false` prevents
cloud-init from touching this file; the hostname comes from DHCP or instance
metadata at runtime.

## Encryption (Metal Variant)

### LUKS

r[image.luks.format]
The root partition must be formatted with LUKS2 using an empty passphrase in
key slot 0.

r[image.luks.keyfile]
An empty keyfile must be installed at `/etc/luks/empty-keyfile` with mode 000.
Dracut must be configured to include this keyfile in the initramfs.

r[image.luks.crypttab]
`/etc/crypttab` must be configured to automatically decrypt the root on boot.

r[image.luks.reencrypt]
The system must rotate the master key of the LUKS volume on first boot,
so that each installation has unique key material.

### TPM Auto-Enrollment

r[image.tpm.service]
A service must run when a TPM device is present and has not yet been
enrolled into the system, which calls the `image.tpm.enrollment` script.

r[image.tpm.enrollment]
A TPM enrollment script must use be configured which binds the LUKS
volume to TPM2 PCR 7, then removes the empty password key slot.
The crypttab must be updated to use the TPM device from then on.

r[image.tpm.disableable]
TPM auto-enrollment must be disableable.

## Output

r[image.output.raw]
A raw disk image file (`.raw`) must be produced, and compressed with zstd.

r[image.output.vmdk]
A VMDK image must be produced.

r[image.output.qcow2]
A qcow2 image must be produced.

r[image.output.checksum]
SHA256 checksums of all output files must be written to a `SHA256SUMS` file.

# Installer

## Configuration File

r[installer.config.location]
The installer must look for a TOML configuration file named
`bes-install.toml`. It searches the following locations in order, using
the first file found:

1. The BESCONF partition (mounted at `/run/besconf/` by a udev rule or
   mount unit in the live environment).
2. `/run/live/medium/bes-install.toml` (the ISO filesystem root, as
   mounted by `live-boot`).
3. `/boot/efi/bes-install.toml` (fallback for manual placement).

> r[installer.config.schema+2]
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
> # Use DHCP-provided hostname instead of a static one.
> # Mutually exclusive with hostname and hostname-template.
> hostname-from-dhcp = true
> # Generate a hostname from a template pattern.
> # Mutually exclusive with hostname and hostname-from-dhcp.
> hostname-template = "tamanu-{hex:6}"
> tailscale-authkey = "tskey-auth-xxxxx"
> ssh-authorized-keys = [
>   "ssh-ed25519 AAAA... admin@example.com",
> ]
> # Plaintext password for the ubuntu user (mutually exclusive with password-hash).
> password = "changeme"
> # Pre-hashed password for the ubuntu user (crypt(3) format, e.g. from mkpasswd).
> # Mutually exclusive with password.
> password-hash = "$6$rounds=4096$..."
> ```
>
> All fields are optional. The `[firstboot]` table and all its fields are
> optional. `password` and `password-hash` are mutually exclusive; if both
> are present the installer must report a validation error. The three
> hostname fields — `hostname`, `hostname-from-dhcp`, and
> `hostname-template` — are mutually exclusive; if more than one is present
> the installer must report a validation error.

r[installer.config.hostname-template]
The `hostname-template` field value is a string containing literal characters
and placeholder expressions enclosed in `{...}`. Supported placeholders:
`{hex:N}` (N-character lowercase hex string, 1 <= N <= 32) and `{num:N}`
(N-digit zero-padded decimal string, 1 <= N <= 10). The template must
contain at least one placeholder, literal portions must consist only of
`[a-z0-9-]`, the template must not start or end with a hyphen, and the
fully expanded hostname must not exceed 63 characters. Values are generated
from a cryptographically secure random source.

## Operating Modes

r[installer.mode.interactive]
When no configuration file is found, the installer must launch a fully
interactive TUI with sensible defaults (variant `metal`, disk strategy
`largest-ssd`).

r[installer.mode.prefilled]
When a configuration file is present but `auto` is false or absent, the
installer must launch the TUI with values from the file pre-filled as
defaults. The user can override any value.

> r[installer.mode.auto+2]
> When `auto = true` and all required fields are present, the installer must
> proceed without any interactive prompts. It must:
>
> 1. Log its configuration to the console.
> 2. Display progress during image writing.
> 3. Apply first-boot configuration.
> 4. Reboot automatically on success.
> 5. Print an error and exit with a non-zero status on failure.
>
> Required fields: `variant`, `disk`. Additionally, when `variant` is
> `"metal"`, at least one hostname strategy must be specified:
> `firstboot.hostname`, `firstboot.hostname-from-dhcp = true`, or
> `firstboot.hostname-template`.

r[installer.mode.auto-incomplete+2]
When `auto = true` but required fields are missing (`variant`, `disk`, or
a hostname strategy for the metal variant), the installer must print an error
describing the missing fields and fall back to interactive mode.

## Testing Flags

r[installer.no-reboot]
When the `--no-reboot` flag is passed, the installer must not call `reboot`
after a successful installation. Instead it must exit cleanly with status 0.
This is required for container-based testing where reboot is not meaningful.

## Dry-Run / Testing Mode

r[installer.dryrun]
When the `--dry-run` flag is passed, the installer must not perform any
destructive operations (no disk wiping, no image writing, no filesystem
mounting, no rebooting). Instead, after collecting all user decisions —
whether from automatic mode, prefilled mode, or the interactive TUI — it
must write a JSON file (the "install plan") summarising what it *would* do
and exit with status 0.

r[installer.dryrun.output]
The `--dry-run-output <path>` flag specifies the path for the JSON install
plan. If omitted, the plan is written to stdout.

> r[installer.dryrun.schema+2]
> The install plan JSON has the following structure:
>
> ```json
> {
>   "mode": "auto | prefilled | interactive | auto-incomplete",
>   "variant": "metal | cloud",
>   "disk": {
>     "path": "/dev/nvme0n1",
>     "model": "Samsung 980 PRO",
>     "size_bytes": 1000204886016,
>     "transport": "NVMe"
>   },
>   "disable_tpm": false,
>   "firstboot": {
>     "hostname": "server-01",
>     "hostname_from_template": false,
>     "tailscale_authkey": true,
>     "ssh_authorized_keys_count": 2,
>     "password_set": true
>   },
>   "image_path": "/run/live/medium/images/metal-amd64.raw.zst",
>   "config_warnings": []
> }
> ```
>
> The `firstboot` field is `null` when no first-boot configuration is set.
> `tailscale_authkey` is a boolean (true when a key is provided) to avoid
> leaking secrets into test output. `password_set` is a boolean (true when
> a password or password hash is provided). When `hostname-from-dhcp` is
> chosen, `hostname` is the sentinel string `"dhcp"`. When a hostname was
> generated from a template, `hostname_from_template` is `true`; otherwise
> it is `false`.

r[installer.dryrun.devices]
In dry-run mode the installer must still detect real block devices (via
`lsblk`) unless a `--fake-devices <path>` flag is given, in which case
it reads device definitions from a JSON file instead. The JSON file must
be an array of objects with the same fields as the `disk` object in the
install plan (`path`, `model`, `size_bytes`, `transport`), plus an
optional `removable` boolean (default false).

### Scripted TUI Input

> r[installer.dryrun.script]
> When `--input-script <path>` is passed, the TUI must read keypress events
> from a newline-delimited text file instead of the terminal. Each line
> describes a single key event using the following tokens:
>
> - `enter`, `esc`, `tab`, `backspace`, `up`, `down`, `left`, `right`,
>   `space` — named special keys.
> - `type:<text>` — emits one `Char` keypress per character of `<text>`.
> - Lines starting with `#` are comments and must be ignored.
> - Empty lines must be ignored.

r[installer.dryrun.script.headless]
When `--input-script` is used, the TUI must not initialise the real terminal
(no raw mode, no alternate screen). It must process events from the script
file, update state, and — when the script is exhausted — produce the install
plan from whatever screen state was reached. The TUI must not block waiting
for terminal events.

## TUI

r[installer.tui.welcome]
The TUI must open with a welcome screen that displays a description of what
the image is for, contact information, and instructions on how to proceed.
The user presses Enter to proceed to disk selection.

r[installer.tui.disk-detection]
The TUI must detect available block devices and display their device path,
size, model name, and transport type (SSD, HDD, NVMe, USB, etc.).

r[installer.tui.variant-selection]
The TUI must present a choice between the `metal` and `cloud` variants with
a brief description of each.

r[installer.tui.tpm-toggle]
When the `metal` variant is selected, the TUI must offer a toggle to disable
TPM auto-enrollment.

r[installer.tui.hostname+2]
After variant/TPM configuration, the TUI must present a text input screen
for the system hostname. The field may be pre-filled from the configuration
file. When the `metal` variant is selected, a toggle/checkbox below the text
input allows the user to opt into DHCP hostname assignment instead of typing
a static hostname. When the toggle is ON, the text input is visually dimmed
and the screen displays a note explaining that the system will get its
hostname from DHCP. The user can advance when either the toggle is ON or a
non-empty hostname is entered. When the `cloud` variant is selected, the
toggle is not shown and the hostname is optional; if left empty the image's
built-in default hostname (`ubuntu`) is kept and is expected to be overridden
by DHCP or cloud-init at boot. If the configuration file contains a
`hostname-template`, the installer resolves it to a concrete hostname at
startup and pre-fills the text input with the result.

r[installer.tui.tailscale]
After the hostname screen, the TUI must present a text input screen for a
Tailscale auth key. The field may be pre-filled from the configuration file.
The user can leave it empty to skip Tailscale configuration.

r[installer.tui.ssh-keys]
After the Tailscale screen, the TUI must present a multi-line text input
screen for SSH authorized keys (one per line). The field may be pre-filled
from the configuration file. The user can leave it empty to skip SSH key
configuration.

r[installer.tui.password]
After the SSH keys screen, the TUI must present a password input screen for
the `ubuntu` user. The user types a password, then confirms it by typing it
again. Both fields are masked (displayed as asterisks). If the two entries
do not match, the TUI must display an inline error and not advance. If the
field is left empty, the image's existing default password (`bes`, expired)
is kept. When a password is provided via the configuration file (`password`
or `password-hash`), this screen is skipped in prefilled and auto modes.

r[installer.tui.confirmation]
Before writing, the TUI must show a summary screen listing: target disk
(path, model, size), chosen variant, TPM enrollment status, and any
first-boot configuration. The summary must clearly state that all data on
the target disk will be destroyed. The user must type an explicit confirmation
(not just press Enter).

r[installer.tui.progress]
During image writing, the TUI must display a progress bar showing bytes
written and estimated time remaining.

r[installer.tui.loop-device]
The installer's TUI and write pipeline must not assume the target device is
real hardware. It must work correctly when targeting a loop device backed by
a sparse file (created via `losetup --partscan`). This means no reliance on
udev events for partition discovery (explicit `partprobe` calls are
acceptable), no transport-type filtering that would reject loop devices, and
no SCSI/ATA-specific ioctls.

## Image Writing

r[installer.write.partitions]
Before writing an image, the installer must wipe all existing filesystem,
RAID, and partition-table signatures from the target disk.
After writing the image, the installer must verify the partition table, and expand the disk and root partition to fit.

r[installer.write.source]
Compressed disk images (`.raw.zst`) must be stored on the ISO filesystem. The
installer must select the correct image for the running CPU architecture and
chosen variant.

r[installer.write.disk-size-check]
Before writing, the installer must read the uncompressed image size from the
zstd frame header and verify that the target disk is at least that large. If
the disk is too small, the installer must refuse to write and report the
image size and disk size in the error message.

r[installer.write.decompress-stream]
The installer must stream-decompress the zstd image directly to the target
block device, avoiding the need to hold the uncompressed image in memory or
on a temporary filesystem.

## First-Boot Configuration

r[installer.firstboot.mount]
After writing the image, the installer must mount the target disk's root
BTRFS partition (subvol `@`) to apply first-boot configuration. For the metal
variant, it must unlock the LUKS volume using the empty keyfile first.

r[installer.firstboot.hostname]
If `hostname` is set (including hostnames generated from a template), the
installer must write it to `/etc/hostname` and add a `127.0.1.1` entry to
`/etc/hosts` on the installed system. If `hostname-from-dhcp` is true, the
installer must write an empty `/etc/hostname` (truncate) and remove any
`127.0.1.1` line from `/etc/hosts`. If neither is set (cloud only), the
installer must leave `/etc/hostname` as-is.

r[installer.firstboot.tailscale-authkey]
If `tailscale-authkey` is set, the installer must write the key to
`/etc/bes/tailscale-authkey` and enable a first-boot systemd service that
runs `tailscale up --auth-key=<key> --ssh`, restricts SSH via UFW, and then
deletes the key file.

r[installer.firstboot.ssh-keys]
If `ssh-authorized-keys` is set, the installer must append each key to
`/home/ubuntu/.ssh/authorized_keys` with correct ownership and permissions
(directory 700, file 600, owned by `ubuntu`).

r[installer.firstboot.password]
If a password is provided (either plaintext or pre-hashed), the installer
must update the `ubuntu` user's password in `/etc/shadow` on the installed
system. When a plaintext password is given, it must be hashed with SHA-512
crypt (`$6$`). When a pre-hashed password is given, it must be written
directly. In either case, the password expiry flag must be cleared so that
the user is not forced to change the password on first login.

r[installer.firstboot.tpm-disable]
If `disable-tpm` is true, the installer must remove the
`setup-tpm-unlock.service` enable symlink from the installed system.

r[installer.firstboot.unmount]
After applying configuration, the installer must cleanly unmount all
filesystems and close any LUKS volumes before prompting for reboot.

## Container Isolation

> r[installer.container.isolation]
> When the installer is run inside a container (e.g. `systemd-nspawn`) for
> integration testing, it must never have access to the host's real block
> devices. Safety is enforced by three layers:
>
> 1. `systemd-nspawn` provides its own `/dev`; host block devices are not
>    present unless explicitly bound in. Only the loop device and its
>    partitions are bound.
> 2. The installer is invoked with `--fake-devices`, which bypasses `lsblk`
>    discovery entirely and presents only the loop device.
> 3. The container runs with `--private-network` to prevent any network
>    side-effects.
>
> A test must verify this property by launching a container without running
> the installer and confirming that no host block devices (e.g. `/dev/sda`,
> `/dev/nvme*`) are visible inside.

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

r[iso.offline]
The live ISO must be fully functional without network connectivity. No
packages or data are downloaded during the installation process.

r[iso.contents]
The ISO must contain the compressed disk images (`.raw.zst`) for all variants
of the ISO's architecture, and the TUI installer binary.

r[iso.boot.uefi]
The ISO must be UEFI-bootable via an El Torito EFI boot catalog. The EFI
boot image is a FAT32 filesystem image containing a GRUB EFI binary at
the default removable media path (`EFI/BOOT/BOOTX64.EFI` for amd64,
`EFI/BOOT/BOOTAA64.EFI` for arm64).

r[iso.boot.autostart]
On boot, the live environment must automatically launch the TUI installer on
the primary console.

r[iso.config-partition]
The ISO must include an appended FAT32 partition (GPT type `Microsoft basic
data`) created via `xorriso --append_partition`. This partition is embedded
in the ISO file and becomes a real writable GPT partition after the ISO is
written to USB via `dd`. Its filesystem label must be `BESCONF`.

When booted from USB, this partition is writable and is the intended location
for users to place a `bes-install.toml` configuration file before booting.
When booted as optical media in a VM, the partition is still readable.

r[iso.per-arch]
Separate ISO images must be produced per architecture (amd64, arm64). Each
ISO contains only the images and installer binary for its architecture.

r[iso.usb]
The ISO must be writable to USB media using `dd` and must boot correctly on
UEFI systems from that media.

# CI/CD

r[ci.shellcheck]
All shell scripts in the repository must pass shellcheck with no errors.

r[ci.unit-test]
Unit tests must be checked in CI.

r[ci.uptodate]
All `uses:` actions must be up to date.

r[ci.rust-stable]
Rustup must be used to install and select the latest stable Rust version.
The dtolnay/rust-toolchain action must not be used.

r[ci.rust-cache]
The "swatinem" rust caching system must be used.

r[ci.output-arch]
CI must produce at least `amd64` and `arm64` outputs.

r[ci.output-suite]
CI must produce images based on Ubuntu Server 24.04 LTS.
