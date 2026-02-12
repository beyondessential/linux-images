## Which image to use

- On AWS: use the AMI
- On other clouds or on-premise virtualisation: use the `cloud` images
- On bare metal: use the `metal` image
- On bare metal where you have a TPM: use the `metal-encrypted` image
- On bare metal where you need to install from USB/DVD: use the ISO

### The disk images

The disk images are pre-prepared volumes containing a working Ubuntu Server Linux system
configured as per BES's preferences for disk and system layout. Select the right image
for your environment and CPU architecture, write it to your disk and configure EFI to
boot from it.

#### Networking

The system is configured to block all incoming ports except for SSH and HTTP ports.
Outgoing and forwarding traffic is allowed.

cloud-init user-data is enabled, allowing e.g. cloud images to be configured by
cloud providers for networking purposes at first boot. Otherwise it's assumed a DHCP
network is available.

#### Credentials

Unless overridden with cloud-init, the system has the default password `bes` for both
the `ubuntu` and `root` users. It's expected that an admin installing the system **logs
in as the `ubuntu` user at the console** once the install is done. This will prompt for
a new password to be set. The admin should then run: `sudo su`, which will require a new
password to be set for the `root` user as well.

Passwords are not allowed over SSH, only the local or serial console.

**Once passwords are set up, the admin must run `ts-up` to connect to Tailscale.**
If so provided by BES, an authentication key should be entered at this stage.
Otherwise, pressing Enter will display a link (and a QR code containing that link)
which must be sent to BES for configuration.

Once Tailscale is successfully connected, SSH access will be forbidden outside
of LAN and link-local IP ranges. The only access available remotely will be via
Tailscale. As passwords are not allowed over SSH, if a local admin wants to SSH
access, they must add their SSH public key to the `.ssh/authorized_keys`.
Cloud providers will likely do this automatically via cloud-init.

#### Disk layout

The disk layout is as follows:
- EFI System Partition, FAT32, label `efi`
- Extended Boot Partition, ext4, label `xboot`
- Linux swap space, label `swap`
- Linux system partition, BTRFS over LUKS, label `root`

The filesystem is BTRFS, and has a subvolume-based inner layout, with the `@` subvolume
mounted as `/`, and the `@postgres` subvolume mounted as `/var/lib/postgresql`. Simple
quotas are enabled to track per-subvolume disk usage.

Transparent filesystem compression is enabled system-wide.

The Snapper snapshot manager is enabled by default, which takes snapshots of the subvolumes
regularly and retains them for default 10/7/4/12 periods. This provides a simple way to
rollback a server or a file to an earlier configuration and protects against catastrophes.

The system partition can be grown or shrunk while online. Shrinking is a manual process
which may involve data loss, but growing is performed automatically if more space is
available at boot.

#### For the metal-encrypted variant and the ISO

The swap space is encrypted with a random key at boot. This makes "resume" (hibernation)
impossible — which is fine, as servers shouldn't be hibernated anyway.

The system partition is encrypted with LUKS, and by default has an empty passphrase.
This provides no security as-is, but if a TPM2 device is present or added to the machine,
the system will automatically detect it, write its encryption key to the TPM2, and
remove the empty passphrase. This binds the disk to the machine's TPM2 device, and enables
the promise of a full-disk-encryption unattended server system when such a device is present.

Non-encrypted variants are expected to be installed in environments where encryption at rest
is provided by the storage system in some way, such as hardware encryption, a cloud provider,
or a SAN.

### The ISO image

The ISO is a standard Ubuntu Server install ISO "CD" image that has been customized to
automatically install BES's preferred disk and system layout on boot without any
interaction required from the user. Simply write it to a USB device (or DVD), boot the
target machine from it, and it will proceed with the installation.

The ISO will automatically select the largest SSD, or failing that, the largest hard disk.
It will overwrite any contents, so make sure you're not connecting any storage you mind
losing at this stage.

A minimum of 8GB space is required. If less than that is provided, the install will fail.
There is no early check for minimum space, so it might take a while and not be obvious.

Installation requires internet as additional packages are required during the process.
The network should be a DHCP subnet with outgoing access.

The resulting system is almost identical to one obtained from the disk images, except
that the UUIDs of the disks and partition tables are all unique, and the encryption
master key is randomly generated instead of being fixed.
