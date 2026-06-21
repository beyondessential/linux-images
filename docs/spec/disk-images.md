# Disk Images

## Partition Layout

r[image.partition.table]
The disk image must use a GPT partition table.

r[image.partition.count]
The disk image must contain exactly three partitions: EFI, extended boot, and
root. There is no swap partition.

r[image.partition.efi]
For the `metal` and `cloud` variants, the first partition must be an EFI
System Partition (type UUID `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`),
formatted as FAT32, labeled `efi`, and sized 512 MiB.

r[image.partition.pi-firmware]
For the `pi` variant, the first partition occupies the same role as the EFI
partition above but holds the Raspberry Pi firmware files, kernel, initramfs,
DTBs and overlays read by the Pi 5 EEPROM bootloader. It must be a FAT32
partition (type UUID `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`), labeled
`firmware`, and sized at least 1 GiB. It is mounted at `/boot/firmware`.

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

r[image.btrfs.format+2]
The root partition (or the LUKS volume on top of it, for the encrypted
variant) must be formatted as BTRFS with the label `ROOT`, the xxhash
checksum algorithm, and the `block-group-tree` and `squota` features enabled.

r[image.btrfs.subvolumes]
The BTRFS filesystem must contain two subvolumes: `@` mounted at `/`, and
`@postgres` mounted at `/var/lib/postgresql`.

r[image.btrfs.compression]
All BTRFS mounts must use transparent zstd compression at level 6 via the
`compress=zstd:6` mount option.

r[image.btrfs.quotas]
BTRFS simple quotas must be enabled on the filesystem.

## Variants

> r[image.variant.types+3]
> Three build-time image variants must be supported: `metal`, `cloud`, and
> `pi`.
>
> The `metal` variant encrypts the root partition with LUKS2. It is intended
> for bare-metal and on-premise virtualisation. The image ships with a
> placeholder empty passphrase; the installer is responsible for rotating the
> master key and enrolling the real unlock mechanism (TPM, keyfile, or
> recovery passphrase) at install time.
>
> The `cloud` variant does not encrypt the root partition. It is intended for
> cloud environments where encryption at rest is provided by the
> infrastructure. The ISO installer is built from the cloud image.
>
> The `pi` variant targets Raspberry Pi 5 and is always arm64. It encrypts
> the root partition with LUKS2 (same scheme as `metal`, with an empty
> placeholder passphrase) but boots via the Pi firmware path rather than
> UEFI/GRUB — see r[image.boot.pi-firmware]. There is no installer for the
> `pi` variant; deployment is image-flash to SD/USB/NVMe.
>
> The file `/etc/bes/image-variant` records the disk-encryption mode of the
> running system. Build-time images write the build variant (`metal`,
> `cloud`, or `pi`). For non-Pi images the installer overwrites this with
> the user's chosen encryption mode: `luks-tpm`, `luks-keyfile`, or `plain`.
> Runtime scripts must not assume the file contains only one of those
> values; they should detect the actual situation (e.g. whether LUKS is
> active) instead.

## Base System

r[image.base.debootstrap]
The base system must be debootstrapped into the `@` subvolume.
The debootstrap must create the minimal viable bootable system.

r[image.base.machine-id]
`/etc/machine-id` must be truncated to zero bytes so that systemd generates a
unique machine ID on each first boot. The image's initramfs must also carry an
uninitialized `/etc/machine-id` (zero bytes, or a string systemd recognises as
uninitialized such as the all-zeros UUID): if the initramfs ships a populated
value, systemd commits it to the root filesystem at switch-root, and every
install ends up with the same machine ID.

r[image.base.resolver]
systemd-resolved must be enabled and configured as the system DNS resolver.

r[image.base.network+2]
A netplan configuration must be installed at
`/etc/netplan/01-all-en-dhcp.yaml` that matches all Ethernet interfaces
(`en*`) and enables DHCPv4. This ensures that all images obtain an IP
address on first boot regardless of whether cloud-init or any other
datasource is present.

r[image.base.console-font]
The `console-setup` and `kbd` packages must be installed so that
`systemd-vconsole-setup.service` configures the Linux console with a
readable font at boot. `/etc/default/console-setup` must be present with
`FONTFACE="Fixed"` and `FONTSIZE="8x16"`.

r[image.base.login-banner]
The pre-login banner displayed on every TTY, including the serial
console, must include the host's current IPv4 and IPv6 network
addresses. An operator with serial-only access (no HDMI, no prior
network knowledge) must be able to read the addresses straight off the
banner without logging in first.

## Packages

r[image.packages.bes-tools]
The bes-tools APT repo must be configured. The `bestool` package must be
installed from this repo (it is exclusively published there). When the OS
archive of a given suite ships an outdated version of any other required
package, the bes-tools repo must be the source for that package; otherwise
the OS archive is preferred.

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

r[image.packages.chrony]
Chrony must be installed and enabled as the system time synchronization
daemon. No other time synchronization daemon (in particular,
systemd-timesyncd) may be active on the running image.

## Bootloader

r[image.boot.dracut]
The shipped image's initramfs must be portable across hardware — it is not
yet bound to a specific machine, so it must contain the kernel modules for
the hardware classes the image is intended to run on (see
`image.boot.hardware-drivers` and `image.boot.cloud-drivers`). The installer
specialises the initramfs to the target machine after install (see
`installer.write.rebuild-boot-config`).

> r[image.boot.hardware-drivers+3]
> The initramfs must contain kernel modules for hardware not present at
> image-build time. The following module categories must be present:
>
> - **NVMe:** `nvme`, `nvme_core`
> - **SATA/AHCI:** `ahci`
> - **RAID controllers:** `megaraid_sas`, `mpt3sas`
> - **Virtio (KVM/Proxmox/GCP/OpenStack):** `virtio_blk`, `virtio_scsi`,
>   `virtio_net`, `virtio_pci`
> - **Intel Ethernet:** `e1000e`, `igb`, `ixgbe`, `i40e`, `ice`
> - **Broadcom Ethernet:** `bnxt_en`, `tg3`
> - **Mellanox/NVIDIA Ethernet:** `mlx5_core`
> - **USB storage:** `usb_storage`, `uas`
> - **Hyper-V:** `hv_storvsc`, `hv_netvsc`, `hv_vmbus`

> r[image.boot.cloud-drivers+5]
> The cloud variant's initramfs must additionally contain cloud-specific
> kernel modules:
>
> - **AWS:** `ena`, `xen_blkfront`
> - **GCP:** `gve`

r[image.boot.grub-install]
For the `metal` and `cloud` variants, GRUB must be installed as the EFI
bootloader with `--bootloader-id=ubuntu`.

r[image.boot.grub-timeout]
GRUB must be configured with `GRUB_TIMEOUT=5`,
`GRUB_TIMEOUT_STYLE=menu`, and `GRUB_RECORDFAIL_TIMEOUT=5`.

r[image.boot.grub-cmdline]
The GRUB kernel command line must include `noresume`.

r[image.boot.cloud-console]
The cloud variant must append `console=ttyS0,115200n8` to the GRUB kernel
command line so that boot output is visible on the EC2 serial console (and
equivalent serial consoles on other cloud providers).

r[image.boot.grub-uuids]
The generated `grub.cfg` must reference filesystem UUIDs that match the
actual on-disk filesystems. Specifically, every `search --no-floppy --fs-uuid
--set=root` directive and every `root=UUID=` kernel parameter in `grub.cfg`
must correspond to a UUID present on one of the image's partitions or
volumes. A mismatch means the system will fail to boot.

> r[image.boot.pi-firmware]
> For the `pi` variant, the bootloader is the Raspberry Pi 5 EEPROM
> firmware. No GRUB is installed. The firmware partition (mounted at
> `/boot/firmware`) must contain `config.txt`, the Pi-specific DTB
> (`bcm2712-rpi-5-b.dtb`) and its overlays, and a kernel + initramfs
> pair selected by `config.txt`.

r[image.boot.pi-cmdline]
For the `pi` variant, kernel command-line arguments are read from
`/boot/firmware/cmdline.txt`. The cmdline must reference the LUKS-mapped
root device (`root=/dev/mapper/root`) and the BTRFS subvolume
(`subvol=@,compress=zstd:6`).

r[image.boot.pi-firmware-update]
For the `pi` variant, kernel, initramfs, DTB and overlay updates must be
propagated to the firmware partition on every kernel package upgrade,
without operator action. The propagation must not overwrite the running
known-good boot assets — see r[image.boot.pi-tryboot-rollback].

> r[image.boot.pi-tryboot-rollback]
> For the `pi` variant, the firmware partition must implement an A/B
> boot layout: new kernel/initramfs/DTB assets are staged separately
> from the running known-good set, and a single failed boot of the new
> assets must automatically roll back to the previous known-good set on
> the next boot, with no operator intervention. A subsequent successful
> boot of the new assets must promote them to known-good.
>
> The EEPROM firmware must be recent enough to support the trial-boot
> mechanism. On Pi 5 / 500 / CM5 the floor is firmware dated
> `2025-02-11` or later.

r[image.boot.pi-uart]
For the `pi` variant, the kernel console must be available on the Pi 5
dedicated debug UART connector at 115200 baud, so a USB-TTL adapter on
that connector is the supported headless console. Pi 5 deployments are
unlikely to have HDMI attached.

r[image.boot.pi-peripherals]
For the `pi` variant, I2C and SPI must be enabled at the device-tree level
(`dtparam=i2c_arm=on` and `dtparam=spi=on` in `config.txt`), and the
`i2c-tools` package must be installed for userspace access.

r[image.boot.pi-tpm-overlay]
For the `pi` variant, the `tpm-slb9670` device-tree overlay must be enabled
in `config.txt` so that an Infineon SLB9670 SPI TPM 2.0 module on a HAT is
picked up automatically when present. With no TPM attached, the kernel
probes SPI, sees no response, and skips, leaving boot unaffected.

r[image.boot.pi-pcie-gen3]
For the `pi` variant, the Pi 5's onboard PCIe x1 lane must be set to gen 3
(`dtparam=pciex1_gen=3` in `config.txt`) for full NVMe HAT throughput, and
the boot splash must be disabled (`disable_splash=1`) so the console stays
clean for headless deployments.

> r[image.boot.pi-power-key]
> For the `pi` variant, a systemd-logind drop-in must be installed at
> `/etc/systemd/logind.conf.d/50-bes-power.conf` setting
> `HandlePowerKey=poweroff` — a short press on the Pi 5 power button
> performs a graceful shutdown.
>
> Operators may re-task the button by editing the drop-in (e.g. `reboot`,
> `suspend`, or `ignore` to release the event to a userspace listener on
> `/dev/input/event*`). Holding the button for more than five seconds
> triggers a firmware-level hard cut that the OS cannot intercept.

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
> A systemd service must be installed and enabled with run conditions that
> trigger when either of the following non-empty files exists:
>
> - `/etc/bes/tailscale-authkey` — written by the installer onto the root
>   filesystem.
> - `/boot/firmware/tailscale-authkey` — operator drop-in on the `pi` variant's
>   firmware FAT partition. An operator can write this file from a workstation
>   (mounting the SD card or USB drive before first boot) so the Pi joins the
>   tailnet on first boot without needing console or LAN access.
>
> When triggered, the service must authenticate to tailscale using the auth key
> read from the first of those files that exists, and enable `--ssh`.
>
> If tailscale is already authenticated when the service runs, it must exit
> cleanly without consuming or modifying any auth-key file.
>
> On success the service must delete the key file it consumed and restrict the
> SSH UFW rule to LAN ranges and the `tailscale0` interface (the same firewall
> tightening that `ts-up` performs).
> The service must remain enabled: with neither key file present the service
> is effectively a no-op, and it leaves open the possibility to re-enable
> first-boot auth by adding either file.
>
> On failure the service must log the error but not prevent boot. The service
> must be ordered after `tailscaled.service` and `network-online.target`.

r[image.tailscale.auto-update]
A weekly cron job must be present to run `apt install -y tailscale`.

## First-boot script

> r[image.firstboot.script]
> A systemd service must be installed and enabled that runs at boot when the
> following two conditions both hold:
>
> 1. The marker file `/etc/bes/firstboot-script.done` does not exist.
> 2. At least one of the manifest paths exists:
>    - `/etc/bes/firstboot-script` — installer-staged, on the root filesystem.
>    - `/boot/firmware/firstboot-script` — operator drop-in on the `pi`
>      variant's firmware FAT partition.
>
> The shipped image must include an empty manifest file at
> `/etc/bes/firstboot-script` (every variant) and at
> `/boot/firmware/firstboot-script` (`pi` variant only).
>
> Manifest format (blank lines and lines whose first non-whitespace character
> is `#` are ignored): exactly two meaningful lines, in order:
>
> 1. A URL using either the `http` or `https` scheme.
> 2. A checksum in the form `sha256:<64-hex>`.
>
> A manifest with no meaningful lines (empty or comments-only) must be
> treated as "no script to run": the service writes the marker and returns
> success without fetching anything.
>
> When a manifest with meaningful content exists, the service must download
> the URL, verify that the sha256 digest of the downloaded bytes matches
> the manifest, then execute the downloaded file as root.
>
> The marker `/etc/bes/firstboot-script.done` and the deletion of all
> manifest files must happen after a successful download+checksum (or after
> determining that no manifest has content), and before the downloaded
> file is executed. The marker therefore reflects "this image has had its
> first-boot opportunity consumed", independent of whether the operator's
> script itself succeeded. If the download fails or the checksum does not
> match, neither the marker nor any manifest file may be touched, so a
> subsequent boot retries.
>
> On failure (anywhere in the pipeline) the service must log the error but
> must not prevent boot. The service must be ordered after the tailscale
> first-boot auth service from `r[image.tailscale.firstboot-auth]`, but must
> not require that service to succeed. The service must additionally be
> ordered after `network-online.target` and `local-fs.target`.

## Snapper

r[image.snapper.root]
Snapper must be configured for the root subvolume (`/`) with timeline
snapshots enabled and retention of 6 hourly snapshots, plus 10 numbered
(non-timeline) snapshots. Daily, weekly, monthly, and yearly timeline
retention must be disabled.

r[image.snapper.postgres]
Snapper must be configured for the PostgreSQL subvolume
(`/var/lib/postgresql`) with the same retention settings as the root config.

r[image.snapper.timers]
The `snapper-timeline.timer` and `snapper-cleanup.timer` systemd timers must
be enabled.

## Disk Growth

> r[image.growth.service+3]
> A systemd service `grow-root-filesystem.service` must run early at boot
> (before user sessions, before LUKS re-encryption) to expand the root partition
> and filesystem if additional disk space is available. It must, in order:
>
> 1. Move the GPT secondary header to the end of the disk.
> 2. Expand the root partition to fill available space.
> 3. If LUKS is active, resize the LUKS container.
> 4. Resize the BTRFS filesystem to fill the partition (or LUKS volume).

## Credentials

r[image.credentials.ubuntu-user]
A `ubuntu` user must exist with the pre-set password `bes`. The password must
be marked expired so that console login forces an immediate password change.

r[image.credentials.root-disabled]
The `root` user must have its shell set to `/sbin/nologin`.

r[image.credentials.no-root-ssh]
SSH access for the `root` user must be disabled via `PermitRootLogin no`.

r[image.credentials.ssh-password-auth]
The cloud build-time image must have SSH password authentication disabled;
only key-based authentication is permitted. The metal build-time image must
have SSH password authentication enabled so that the pre-set `ubuntu` user
credentials are usable over SSH.

r[image.credentials.no-host-keys+2]
The image must not contain SSH host keys (`/etc/ssh/ssh_host_*`). The
openssh-server package generates host keys at install time, but these must
be deleted during the image build so that each deployed instance generates
its own unique keys on first boot. Shipping shared host keys is a security
risk (enables MITM) and leaks the build machine's hostname in the key
comment.

r[image.credentials.host-key-regen]
A service must be installed and enabled that regenerates SSH host keys on
first boot when they are missing. The service must run before the SSH
daemon starts and must be conditioned on a host key file not existing so
that it is a no-op on subsequent boots.

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

r[image.hostname.metal-dhcp+2]
The metal build-time image must ship with an empty `/etc/hostname` (zero
bytes) so that `systemd-hostnamed` accepts DHCP-provided transient
hostnames. `/etc/hosts` must contain only `localhost` entries (no
`127.0.1.1` line).

r[image.hostname.cloud-default+2]
The cloud build-time image must ship with `ubuntu` as the static hostname
in `/etc/hostname`. Cloud-init with `create_hostname_file: false` prevents
cloud-init from touching this file; the hostname comes from DHCP or
instance metadata at runtime.

## Encryption (Metal Build Variant)

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
A raw disk image file (`.img`) must be produced, and compressed with zstd.

r[image.output.vmdk]
A VMDK image must be produced.

r[image.output.qcow2]
A qcow2 image must be produced.

r[image.output.checksum]
SHA256 checksums of all output files must be written to a `SHA256SUMS` file.

> r[image.output.aws-ami]
> On a tagged release, the cloud variant image for every (suite, architecture)
> combination that is published must be registered as an AWS AMI in the
> `ap-southeast-2` region. The set of currently-published suites and
> architectures is captured by the build matrix in
> `.github/workflows/build.yml`.
>
> Each registered AMI must be named
> `ubuntu-<ubuntu-version>-bes-cloud-<arch>-<version>`, where
> `<ubuntu-version>` is the numeric Ubuntu release corresponding to the
> suite, `<arch>` is the image architecture, and `<version>` is the release
> version without the leading `v`. AMIs from different suites must therefore
> not collide on a name even when registered from the same release tag.
>
> Each registered AMI must carry the following AWS resource tags: `Name`,
> `Os`, `OsVersion`, `Variant`, `Architecture`, `Version`, `Features`, and
> `Builder`. `OsVersion` must hold the numeric Ubuntu release
> (`<ubuntu-version>` above).
