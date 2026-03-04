## Which image to use

- On AWS: use the AMI
- On other clouds or on-premise virtualisation: use the `cloud` images
- On bare metal: use the `metal` image
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

#### Hostname

The disk images ship with a default hostname of `ubuntu`. What happens to this
hostname depends on the variant and how the image is installed:

- **Metal variant (direct image write, no installer):** the image ships with an
  empty `/etc/hostname`, so the system automatically picks up a DHCP-provided
  hostname. If DHCP does not provide one, `hostnamectl` shows
  `Static hostname: n/a` and the transient hostname is `localhost`.
- **Metal variant (TUI or auto install):** the installer requires a hostname
  strategy. The user can type a static hostname, toggle the "Use DHCP hostname"
  checkbox (Tab/Space on the hostname screen), or use a `hostname-template` in
  the config file. In auto mode, one of `hostname`, `hostname-from-dhcp = true`,
  or `hostname-template` must be present in the `[firstboot]` table.
- **Cloud variant (TUI install):** the hostname is optional in the TUI. If left
  empty, the default `ubuntu` hostname is kept. It is expected to be overridden
  at boot by DHCP or cloud-init metadata from the cloud provider.
- **Cloud variant (direct image write, no installer):** the image boots with
  hostname `ubuntu`. Cloud providers typically override this via cloud-init
  instance metadata or DHCP.

#### Timezone

The system timezone defaults to `UTC`. When installing via the ISO Installer,
the TUI presents a searchable timezone selection screen after the password
screen. The list is populated from the system's IANA timezone database
(`/usr/share/zoneinfo/zone1970.tab`). The user can type to filter the list
and use Up/Down arrows to navigate. Pressing Enter selects the highlighted
timezone.

The timezone can also be set in the `bes-install.toml` configuration file:

```
[firstboot]
timezone = "Pacific/Auckland"
```

If not specified, the timezone defaults to `UTC`. The installer writes the
selected timezone by creating a symlink at `/etc/localtime` pointing to the
corresponding file under `/usr/share/zoneinfo/` and writing the timezone
name to `/etc/timezone`.

#### Credentials

The `ubuntu` user is the only login account. The `root` user has no password and
its shell is set to `/sbin/nologin`, so direct root login is not possible. The
`ubuntu` user has passwordless `sudo` access.

When installing via the ISO Installer, the TUI prompts for a password for the
`ubuntu` user. If left empty, the image's default password (`bes`, expired) is
kept and the user will be forced to change it on first console login. A password
can also be set in the `bes-install.toml` configuration file:

```
[firstboot]
# Plaintext (mutually exclusive with password-hash):
password = "changeme"
# Or pre-hashed (crypt(3) format, e.g. from mkpasswd):
password-hash = "$6$rounds=4096$..."
```

Passwords are not allowed over SSH, only the local or serial console.

**Once the password is set, the admin must run `ts-up` to connect to Tailscale.**
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

#### Full disk encryption

When using the `metal` disk image or selecting it from the ISO, the system partition is
encrypted with LUKS, and by default has an empty passphrase. This provides no security as-is,
but if a TPM2 device is present or added to the machine, the system will automatically detect
it, write its encryption key to the TPM2, and remove the empty passphrase. This binds the disk
to the machine's TPM2 device, and enables the promise of a full-disk-encryption unattended
server system when such a device is present.

The `cloud` images don't have encryption, and are expected to be installed in environments
where encryption at rest is provided by the storage system in some way, such as hardware
encryption, a cloud provider, or a SAN.

The TPM automatic binding can also be disabled when installing the `metal` variant with the
ISO's interactive installer; the partition is still encrypted (with an empty passphrase),
and TPM binding can be done manually.

### The ISO image

The ISO is a custom "Live" ISO image that starts a terminal-based "graphical" interface
which allows you to interactively select options, and also has an unattended mode via a
config file when written to a USB device.

A minimum of 5GB space is required. If less than that is provided, the install will refuse
to proceed. You can select a larger disk in the installer.

Installation requires no internet: all installation files are included on the image.
If internet is available (the ISO system will attempt to connect using DHCP), then
additional checks are performed which can help you diagnose common networking issues
even before installing the system. Failing network checks will not prevent an install
from going ahead, they're purely informative.

### The ISO config file

When writing the ISO to a USB device, a partition labelled BESCONF can be mounted from the
USB afterwards. This contains a text file named `bes-install.toml`, which can be edited
in a normal text editor like nodepad. The file can be used to change the defaults of the
interactive installer, and even to switch the installer to an automatic/unattended mode.

Example of an automatic config:

```toml
auto = true
variant = "metal"
disk = "largest-ssd"

[firstboot]
hostname = "server-01"
tailscale-authkey = "tskey-auth-xxxxx"
```

Example using DHCP hostname (metal variant, no static hostname):

```toml
auto = true
variant = "metal"
disk = "largest-ssd"

[firstboot]
hostname-from-dhcp = true
```

Example using a hostname template (generates a unique hostname per install):

```toml
auto = true
variant = "metal"
disk = "largest-ssd"

[firstboot]
hostname-template = "tamanu-{hex:6}"
```

Example of a config setting custom defaults only:

```toml
variant = "cloud"

[firstboot]
hostname = "server-02"
```

### `bes-install.toml` field reference

All fields are optional. Unknown fields are rejected.

#### Top-level fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `auto` | boolean | `false` | Run fully automatically without prompts. Requires `variant` and `disk`; additionally, the `metal` variant requires a hostname strategy (`hostname`, `hostname-from-dhcp`, or `hostname-template`) in the `[firstboot]` table. |
| `variant` | string | — | Image variant to install. `"metal"` for full-disk encryption (LUKS2) with optional TPM auto-unlock, or `"cloud"` for no encryption (intended for environments with host-level disk encryption). |
| `disk` | string | — | Target disk for installation. Either a device path (e.g. `"/dev/sda"`) or a selection strategy: `"largest-ssd"` (largest SSD by capacity), `"largest"` (largest disk of any type), or `"smallest"` (smallest disk of any type). |
| `disable-tpm` | boolean | `false` | Disable automatic TPM2 enrollment on first boot. Only meaningful with the `metal` variant; ignored (with a warning) for `cloud`. The LUKS volume is still created but will not be bound to the TPM. |

#### `[firstboot]` table

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `hostname` | string | — | Hostname to set on first boot. Required for the `metal` variant unless `hostname-from-dhcp` or `hostname-template` is used; optional for `cloud` (defaults to `ubuntu`, typically overridden by DHCP/cloud-init). Must be 1--63 characters, containing only ASCII alphanumerics and hyphens, and must not start or end with a hyphen. Mutually exclusive with `hostname-from-dhcp` and `hostname-template`. |
| `hostname-from-dhcp` | boolean | `false` | Use the DHCP-provided hostname instead of a static one. When enabled, `/etc/hostname` is left empty so that `systemd-hostnamed` accepts the transient hostname from DHCP. Mutually exclusive with `hostname` and `hostname-template`. |
| `hostname-template` | string | — | Generate a unique hostname from a template pattern. The template contains literal characters and `{hex:N}` or `{num:N}` placeholders (e.g. `"tamanu-{hex:6}"` produces `"tamanu-a3f1b2"`). Must contain at least one placeholder; literals must be `[a-z0-9-]`; result must not exceed 63 characters. Mutually exclusive with `hostname` and `hostname-from-dhcp`. |
| `tailscale-authkey` | string | — | Tailscale authentication key (e.g. `"tskey-auth-xxxxx"`) used to automatically join the Tailscale network on first boot. |
| `ssh-authorized-keys` | array of strings | `[]` | SSH public keys to install for the default user. Each entry must be a non-empty SSH public key string (e.g. `"ssh-ed25519 AAAA... admin@example.com"`). |
| `password` | string | — | Plaintext password for the `ubuntu` user. Hashed with SHA-512 crypt and written to `/etc/shadow` on the installed system, with the expiry flag cleared. Mutually exclusive with `password-hash`. |
| `password-hash` | string | — | Pre-hashed password for the `ubuntu` user in crypt(3) format (e.g. from `mkpasswd --method=sha-512`). Written directly to `/etc/shadow` with the expiry flag cleared. Mutually exclusive with `password`. |
| `timezone` | string | `"UTC"` | IANA timezone for the installed system (e.g. `"Pacific/Auckland"`, `"America/New_York"`). The installer creates a symlink at `/etc/localtime` pointing to `/usr/share/zoneinfo/<timezone>` and writes the name to `/etc/timezone`. In the TUI, a searchable list of timezones is presented; type to filter and use Up/Down to navigate. |
