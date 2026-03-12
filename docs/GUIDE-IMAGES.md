# Guide for BES Linux direct-to-disk images

The disk images are pre-prepared volumes containing a working Ubuntu Server Linux system
configured as per BES's preferences for disk and system layout.
Select the right image for your environment and CPU architecture, write it to your disk and configure EFI to boot from it.

## Checksums

SHA256 checksums are provided by GitHub in the releases page.

Please verify images before writing them to disks.

## Version

The images are based on **Ubuntu Server 24.04 LTS**.

Ubuntu Server 26.04 LTS support is planned for mid-2026.
Non-LTS versions e.g. 25.10 will not be supported.

## Boot

UEFI is required.
We will not add "BIOS" / non-UEFI support.

### Encryption

Cloud images have a plaintext system partition.

Metal images have an encrypted system partition.
They will rotate their volume master key on first boot, which will add a few minutes to the boot process.

The encrypted partition is configured with an empty password.
This provides little security immediately, but can be trivially bound to a TPM or other hardware key later.
It also means that wiping a disk securely is cheap (erase the first megabyte of the partition, "destroying" the data by losing the volume master key).

### Firmware

Cloud images only have generic Linux modules, supporting basic x86/ARM64 hardware and paravirtualisation (virtio, KVM, etc).

Metal images ship with the full set of Linux firmware and driver modules, supporting more exotic storage and networking configurations.

## Networking

**cloud-init user-data** is enabled, allowing e.g. cloud images to be configured by cloud providers for networking purposes at first boot.
Otherwise it's assumed a DHCP network is available.

### Firewall

The system is configured to block all incoming ports except for SSH and HTTP ports.
Outgoing and forwarding traffic is allowed.

### Hostname

The direct-to-disk images ship with **no default hostname** and **cloud-init enabled**.
This lets you assign a hostname either through cloud-init (if deploying in a compatible cloud or on-prem virtualisation system) or through DHCP (matching on the MAC address).

If neither of these are available, you should set the hostname manually.
Some disk imaging software will let you customise the /etc/hostname file directly from the imager, though that is only likely to work with the "cloud" image.
Otherwise, use `sudo hostnamectl set-hostname yourhostname` on first login.

## Timezone

The system timezone is set to `UTC`.

**cloud-init** may be used to configure this on first boot.

Otherwise, set the timezone using `timedatectl set-timezone Australia/Melbourne`.

## Login

The `ubuntu` user is the only login account.
It has passwordless `sudo` access.

The initial password is `bes`.
You will be prompted to change it at first login.

### Root account

The `root` user has no password and its shell is set to `/sbin/nologin`, so direct root login is not possible.

## Tailscale

Tailscale is pre-installed.

You can run `ts-up` once logged in to connect Tailscale.
This will ask for an "authkey", which you can paste in if you have one.
Otherwise, it will print a URL and a QR code (useful when you can't copy text from the console) to do interactive authentication; send it to BES (or use it directly if you're connecting to your own Tailscale account).

Once Tailscale is successfully connected, SSH access will be forbidden outside of LAN and link-local IP ranges.
The only access available remotely will be via Tailscale.

## Disk layout

The disk layout is as follows:
- 512MB EFI System Partition, FAT32, label `efi`
- 1GB Extended Boot Partition, ext4, label `xboot`
- 4+GB Linux system partition, BTRFS, label `root`

The filesystem is BTRFS, and has a subvolume-based inner layout, with the `@` subvolume mounted as `/`, and the `@postgres` subvolume mounted as `/var/lib/postgresql`.
Simple quotas are enabled to track per-subvolume disk usage.

Transparent filesystem compression is enabled system-wide.

The Snapper snapshot manager is enabled by default, which takes snapshots of the subvolumes hourly and retains them for default 10 hours / 7 days / 4 weeks / 12 months periods.
This provides a simple way to rollback a server or a file to an earlier configuration and protects against catastrophes.

The system partition can be grown or shrunk while online.
Shrinking is a manual process, but growing is performed automatically if more space is available at first boot.
