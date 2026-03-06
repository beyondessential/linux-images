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

> r[image.variant.types+2]
> Two image variants must be supported: `metal` and `cloud`.
>
> The `metal` variant encrypts the root partition with LUKS2. It is intended
> for bare-metal and on-premise virtualisation. The image ships with a
> placeholder empty passphrase; the installer is responsible for rotating the
> master key and enrolling the real unlock mechanism (TPM, keyfile, or
> recovery passphrase) at install time.
>
> The `cloud` variant does not encrypt the root partition. It is intended for
> cloud environments where encryption at rest is provided by the infrastructure.
>
> The active variant name must be written to `/etc/bes/image-variant` in the
> installed system so that runtime scripts can branch on it.

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

r[image.boot.grub-uuids]
The generated `grub.cfg` must reference filesystem UUIDs that match the
actual on-disk filesystems. Specifically, every `search --no-floppy --fs-uuid
--set=root` directive and every `root=UUID=` kernel parameter in `grub.cfg`
must correspond to a UUID present on one of the image's partitions or
volumes. A mismatch means the system will fail to boot.

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

> r[image.tailscale.firstboot-auth]
> A systemd service must be installed and enabled with a run condition for the file `/etc/bes/tailscale-authkey` existing.
> It must authenticate the server to tailscale using the authkey in the file, and enable `--ssh`.
>
> On success the service must delete the key file and restrict the SSH UFW rule to LAN ranges and the `tailscale0` interface (the same firewall tightening that `ts-up` performs).
> The service must remain enabled: the key file not existing will effectively disable it, and it leaves open the possibility to re-enable by adding the file.
>
> On failure the service must log the error but not prevent boot. The service must be ordered after `tailscaled.service` and `network-online.target`.

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

## Output

r[image.output.raw]
A raw disk image file (`.raw`) must be produced, and compressed with zstd.

r[image.output.vmdk]
A VMDK image must be produced.

r[image.output.qcow2]
A qcow2 image must be produced.

r[image.output.checksum]
SHA256 checksums of all output files must be written to a `SHA256SUMS` file.
