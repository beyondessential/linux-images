# Guide for BES Linux ISO/USB installer

[![Animated GIF demoing the entire process](./images/demo.gif)](https://asciinema.org/a/ZlN92tFyAEpc7cun)

The installer is a custom terminal graphics ("TUI") application which runs from a CD/DVD image or bootable USB.
It asks setup questions, writes Linux to the selected disk, and then configures the system.
Unlike the standard Ubuntu Server installer, it always writes a fixed partition layout which matches our direct-to-disk images.

A minimum of 5GB space is required. If less than that is provided, the install will refuse
to proceed. You can select a larger disk in the installer.

Installation requires no internet: all installation files are included on the image.

## Checksums

SHA256 checksums are provided by GitHub in the releases page.

The installer image also embeds checksums for its own data, and verifies them on boot.
This makes it very unlikely that corrupt images will be able to proceed with an install, even if you didn't check the sums before/after writing to USB.
However, as the checksums are embeded in the image, it doesn't protect against malicious tampering.

## Version

The images are based on **Ubuntu Server 24.04 LTS**.

Ubuntu Server 26.04 LTS support is planned for mid-2026.
Non-LTS versions e.g. 25.10 will not be supported.

## Boot

UEFI is required.
We will not add "BIOS" / non-UEFI support.

## Configuration

There are two ways of using the installer: interactive (the default) or automatic.

When using a USB device, a FAT32 partition called BESCONF is available, writeable from any operating system including Windows.
Within that partition is a file named `bes-installer.toml`, which can be used to:
- change defaults for the interactive mode, or
- configure automatic installations.

Descriptions of every option is available directly in the file, and also in this guide.

## Interactive

In the following sections, note that screenshots may be slightly outdated or not match exactly what you're seeing.
The version you're using might not be the same one as we used to generate the images!

### Welcome

On the welcome screen, you have some basic information about the image.
Note how the controls are shown at the bottom of the screen.
In general, "Enter" goes to the next screen, "Esc" goes back, and "Tab" navigates between fields.
You can hit "Ctrl+Alt+d" at any point to get a shell; exit the shell to get back to the installer.

![Console interface with four "windows" vertically stacked: a titlebar reading "BES Installer -- Welcome" and some build time information (date, architecture, commit ID); a main window with explanation text ("Tamanu Linux is BES's preferred system layout..."); a progress bar titled "Verification in progress..." (currently in the middle of checking the disk integrity); a menu bar ("Enter: start, q: reboot, Ctrl+Alt+d: shell").](./images/01-welcome.png)

This is also when we verify all the data on the USB to make sure none of it is corrupted.
On USB2 devices, this might take several minutes, due to needing to read about 4 gigabytes of data.
Prefer using USB3 devices; if you're installing in a virtual machine, change the virtual controller type to "virtio" or similar.
For example, in VirtualBox:

![VirtualBox Storage configuration window in Expert mode, showing the "Removable" controller which has a "bes-installer-amd64.iso" file loaded as a CD image. In its attributes, the Type of the controller is set to "virtio-scsi".](./images/01-virtualbox.png)

#### BESCONF

Every screen will also have a section here describing fields in the configuration file that apply to their settings.
All fields are also documented together in a table at the bottom of this file.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `auto` | boolean | `false` | Run fully automatically without prompts. All other fields are optional and fall back to their defaults. |
| `copy-install-log` | boolean | `true` | Copy the installer log into the installed system at `/var/log/bes-installer.log`. Set to `false` to disable. No TUI control for this option. |

### Network Configuration

There are two separate network configurations you can set: the one for the current "live" system (the installer's own network), and the one that will be configured on the target machine.
For example, you might want to use DHCP for the installer, but configure a static IP for the final install.
By default, the installation target inherits the settings from the installer.

![Console interface with four "windows" vertically stacked: a titlebar reading "BES Installer -- Network Configuration"; a "Live ISO (current)" main window with four radio buttons for "DHCP", "Static IP", "IPv6 SLAAC only", "Offline"; a collapsed window "Installation Target [Tab to expand]"; and the menu bar](./images/02-network-current.png)

#### Live ISO (current)

Use arrow keys to select one of:
- DHCP
- Static IP
- IPv6 SLAAC only
- Offline

If Static IP is selected, then additional fields will appear to be filled in, use Tab to navigate between them:
- Interface: this has a dropdown of all available interface names
- IP address: the static IP in CIDR format. If you don't include a prefix length, it will default to /24.
- Gateway
- DNS: optional
- Search domain: optional

As you change details, a `Status:` line above the form will update, showing what the current network status is:
- Unknown
- No connectivity
- Connected (with the current IP and interface in use)

It can take a little while for DHCP to get configured, so be patient :)

The installer _can_ work fully offline, so you can set it explicitly offline if you want, or not connect a network cable at all.

When done, hit Tab to get to the next pane:

#### Installation Target

![Console interface with four "windows" vertically stacked: a titlebar reading "BES Installer -- Network Configuration"; a collapsed window "Live ISO (current)"; a main window "Installation Target" with five radio buttons for "Copy current config", "DHCP", "Static IP", "IPv6 SLAAC only", "Offline"; and the menu bar](./images/02-network-target.png)

Use arrow keys to select one of:
- Copy current config
- DHCP
- Static IP
- IPv6 SLAAC only
- Offline

The options are the same as for the other pane, except for "Copy current config".
There is no status display for this pane, so make sure you get it right if you're entering Static IP details!

#### Network check

Pressing Alt+c will open a "network check" screen, which tests connectivity to a set of targets.
These are requirements for installing and running Tamanu, so this lets you make the required changes e.g. to your firewalls upfront and have immediate feedback.
Press `r` to run the checks again.

![](./images/02-netcheck.png)

This also performs a `tailscale netcheck`, and you can hit Tab to expand its output to see what the problem is.
If Tailscale is not going to be used for this install, you can of course ignore it.

![](./images/02-tailscale.png)

Network connectivity is not a requirement for the installer, so the network checks failing will not prevent you from proceeding.

Hit Escape to get back to the Network configuration screen.

### Target Disk

All disks will be shown, with their hardware type, device path, and size.
Use arrow keys to select one.

![](./images/03-disk.png)

There are no partitioning options: we will use the entire disk and configure it in a fixed manner.

#### BESCONF

```toml
disk = "smallest"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `disk` | string | `"largest-ssd"` | Target disk for installation. Either a device path (e.g. `"/dev/sda"`) or a selection strategy: `"largest-ssd"` (largest SSD by capacity; the default), `"largest"` (largest disk of any type), or `"smallest"` (smallest disk of any type). |

### Disk Encryption

Use arrow keys to select whether to encrypt the disk, and how to unlock it if so.

![](./images/04-encryption.png)

- Full-disk encryption, not bound to hardware
- No encryption
- Full-disk encryption, bound to hardware \[experimental]

The default is to encrypt the system partition, and store the unlock "keyfile" on an unencrypted partition nearby.
This provides little security on a day to day basis, but means that:
- You can cheaply wipe the data off the disk securely (cryptographic erase)
- You can enable more security later by changing the unlock method

If you choose "no encryption", you can't later re-encrypt the disk without downtime or risk of data loss.

An experimental "bound to hardware" mode is available.
This will attempt to use TPM2 hardware module, if present, to decrypt the disk.
If selected, no keyfile is stored in plaintext on the disk.
However, if unlock fails on reboot, you will need to enter the recovery passphrase to boot, and you may need to manually configure keyfile unlock.
No support is provided for this mode.

Regardless, if encryption is selected, a recovery passphrase is generated and printed to the screen twice: first before the final confirmation, and then after the install is done.
It will not be available again, so make sure to note it down and store that securely.

#### BESCONF

```toml
disk-encryption = "none"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `disk-encryption` | string | `"keyfile"` | Disk encryption mode. `"tpm"` for LUKS + TPM PCR 1 (requires a TPM; experimental), `"keyfile"` for LUKS + keyfile on boot partition (default), or `"none"` for no encryption. |

### Hostname

Use arrow keys to select:
- Static hostname, which will prompt you to enter one
- Network-assigned

![](./images/05-hostname.png)

If you select "Network-assigned", which is the default, the system will not have a hostname set (it will show up as 'localhost` if anything).
DHCP can configure hostnames on such hosts, matching against their MAC address.
This makes it possible to bind the hostname to the hardware rather than the operating system configuration.
Alternatively, cloud-init is available and enabled, so it may provide the hostname that way.

#### BESCONF

A static hostname:

```toml
hostname = "server-02"
```

A hostname template (this can only be set using the configuration file):

```toml
hostname-template = "server-{hex:6}"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `hostname-from-dhcp` | boolean | `true` | Use the DHCP-provided hostname instead of a static one. When enabled, `/etc/hostname` is left empty so that `systemd-hostnamed` accepts the transient hostname from DHCP. |
| `hostname-template` | string | — | Generate a unique hostname from a template pattern. The template contains literal characters and `{hex:N}` or `{num:N}` placeholders (e.g. `"tamanu-{hex:6}"` produces `"tamanu-a3f1b2"`). Must contain at least one placeholder; literals must be `[a-z0-9-]`; result must not exceed 63 characters. Setting it overrides `hostname-from-dhcp`. |
| `hostname` | string | — | Hostname to set during installation. When omitted (and no other hostname strategy is set), the system uses the DHCP-assigned hostname. Must be 1--63 characters, containing only ASCII alphanumerics and hyphens, and must not start or end with a hyphen. Setting it overrides `hostname-from-dhcp` and `hostname-template`. |

### Login

You can't select the username.
This is always set to `ubuntu` for consistency.
You must enter a login password for that user.

![](./images/06-login.png)

You must set a reasonably strong password for production systems.
Please don't use `test` or `Zxc,./2025` or `hunter2`.

This is also where you can enter a Tailscale Authkey if you have one, using Alt+t, or set SSH keys for the `ubuntu` users, using Alt+s.
Typing correct authkeys or SSH keys without copy-paste is difficult, so if you have your public keys in GitHub, you can also enter your username using Alt+g and we'll fetch the keys (requires network).

#### BESCONF

Set a password:

```toml
password = "orca-passage-story-8199"
```

Set a password in pre-hashed form (so it's not revealed to someone picking up the USB drive):

```bash
$ mkpasswd --method=sha-512
```

```toml
password-hash = "$6$rounds=4096$..."
```

Add SSH keys:

```toml
ssh-authorized-keys = [
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIA3mjq5QvWZzP73fbFqpD/TZ++n8S8JzhmTIwMHZ6rG",
    "ssh-rsa AAAAB3NzaC1yc2EAAAABIwAAAQEAklOUpkDHrfHY17SbrmTIpNLTGK9Tjom/BWDSUGPl+nafzlHDTYW7hdI4yZ5ew18JH4JW9jbhUFrviQzM7xlELEVf4h9lFX5QVkbPppSwg0cda3Pbv7kOdJ/MTyBlWXFCR+HAo3FXRitBqxiX1nKhXpHAZsMciLq8V6RjsNAQwdsdMFvSlVK/7XAt3FaoJoAsncM1Q9x5+3V0Ww68/eIFmb1zuUFljQJKprrX88XypNDvjYNby6vw/Pb0rwert/EnmZ+AW4OZPnTPI89ZPmVMLuayrD2cE86Z/il8b+gw3r3+1nKatmIkjn2so1d01QraTlMqVSsbxNrRFi9wrf+M7Q== schacon@mylaptop.local"
]
```

Set the tailscale authkey:

```toml
tailscale-authkey = "tskey-auth-xxxxxxxxxxxx-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `password` | string | — | Plaintext password for the `ubuntu` user. Hashed with SHA-512 crypt and written to `/etc/shadow` on the installed system, with the expiry flag cleared. |
| `password-hash` | string | — | Pre-hashed password for the `ubuntu` user in crypt(3) format (e.g. from `mkpasswd --method=sha-512`). Written directly to `/etc/shadow` with the expiry flag cleared. Overrides `password` if set. |
| `tailscale-authkey` | string | — | Tailscale authentication key (e.g. `"tskey-auth-xxxxx"`). If tailscale netcheck passed during installation, the installer attempts to authenticate directly by chrooting into the target system. If that doesn't run or fails, the key is written to `/etc/bes/tailscale-authkey` for first-boot authentication. |
| `ssh-authorized-keys` | array of strings | `[]` | SSH public keys to install for the default user. Each entry must be a non-empty SSH public key string (e.g. `"ssh-ed25519 AAAA... admin@example.com"`). |

### Timezone

The system timezone defaults to `UTC`.

![](./images/07-timezone.png)

You can select the correct timezone using arrow keys or by typing a search string.

Tamanu has its own timezone settings at the application level and does not need the system to be in a specific timezone, or UTC for that matter.
We do recommend using the local timezone just so it's easier to relate to times while troubleshooting; some people prefer to set all servers to UTC and that's fine too.

#### BESCONF

```toml
timezone = "Australia/Melbourne"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `timezone` | string | `"UTC"` | IANA timezone for the installed system (e.g. `"Pacific/Auckland"`, `"America/New_York"`). The installer creates a symlink at `/etc/localtime` pointing to `/usr/share/zoneinfo/<timezone>` and writes the name to `/etc/timezone`. In the TUI, a searchable list of timezones is presented; type to filter and use Up/Down to navigate. |

### Network check

We show you the network check results again in case you hadn't consulted them previously.
Network misconfiguration is the number one cause of installation issues and delay, so it's worth getting it right upfront.

![](./images/08-netresults.png)

### Confirmation

Once all details are entered, and just before we start writing to disk, we shown a summary of options chosen.

![](./images/09-confirm.png)

We also show the generated recovery passphrase.
While it will be shown again at the end of the process, it's better to note it down right now.

You need to type in `yes` and press Enter (not `y` or just pressing Enter with nothing typed in) to confirm.

### Progress

A progress bar and the writing speed will be displayed while the installer writes data:

![](./images/10-progress.png)

At the end, a few additional steps are performed:

![](./images/10-steps.png)

Finally, the install will be done, and show a final screen.
If you have encryption enabled, the recovery passphrase will be shown one last time.

![](./images/10-final.png)

## Automatic install

If you set `auto = true` in the BESCONF config, the installer will attempt an automatic install.
That means it will not prompt you or even show the interactive interface at all, and perform the install using its defaults or the settings you provide in the file.

This does also mean that there is no recourse if you have a disk in a server with data and it picks it for install: it will happily overwrite any data present without pause.
However, when setting up a large number of servers, or when typing at the console is not convenient, this can be very useful.
We do recommend testing it first, though.

| `auto` | boolean | `false` | Run fully automatically without prompts. All other fields are optional and fall back to their defaults. |

The simplest automatic config is just:

```toml
auto = true
```

With a static hostname and Tailscale:

```toml
auto = true
hostname = "server-01"
tailscale-authkey = "tskey-auth-xxxxx"
```

With a hostname template (generates a unique hostname per install):

```toml
auto = true
hostname-template = "tamanu-{hex:6}"
```

## BESCONF reference

Example of a config setting custom defaults only (not automatic):

```toml
disk-encryption = "none"
hostname = "server-02"
```

All fields are optional. Unknown fields are rejected.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `auto` | boolean | `false` | Run fully automatically without prompts. All other fields are optional and fall back to their defaults. |
| `disk-encryption` | string | `"keyfile"` | Disk encryption mode. `"tpm"` for LUKS + TPM PCR 1 (requires a TPM; experimental), `"keyfile"` for LUKS + keyfile on boot partition (default), or `"none"` for no encryption. |
| `disk` | string | `"largest-ssd"` | Target disk for installation. Either a device path (e.g. `"/dev/sda"`) or a selection strategy: `"largest-ssd"` (largest SSD by capacity; the default), `"largest"` (largest disk of any type), or `"smallest"` (smallest disk of any type). |
| `copy-install-log` | boolean | `true` | Copy the installer log into the installed system at `/var/log/bes-installer.log`. Set to `false` to disable. No TUI control for this option. |
| `hostname-from-dhcp` | boolean | `true` | Use the DHCP-provided hostname instead of a static one. When enabled, `/etc/hostname` is left empty so that `systemd-hostnamed` accepts the transient hostname from DHCP. |
| `hostname-template` | string | — | Generate a unique hostname from a template pattern. The template contains literal characters and `{hex:N}` or `{num:N}` placeholders (e.g. `"tamanu-{hex:6}"` produces `"tamanu-a3f1b2"`). Must contain at least one placeholder; literals must be `[a-z0-9-]`; result must not exceed 63 characters. Setting it overrides `hostname-from-dhcp`. |
| `hostname` | string | — | Hostname to set during installation. When omitted (and no other hostname strategy is set), the system uses the DHCP-assigned hostname. Must be 1--63 characters, containing only ASCII alphanumerics and hyphens, and must not start or end with a hyphen. Setting it overrides `hostname-from-dhcp` and `hostname-template`. |
| `password` | string | — | Plaintext password for the `ubuntu` user. Hashed with SHA-512 crypt and written to `/etc/shadow` on the installed system, with the expiry flag cleared. |
| `password-hash` | string | — | Pre-hashed password for the `ubuntu` user in crypt(3) format (e.g. from `mkpasswd --method=sha-512`). Written directly to `/etc/shadow` with the expiry flag cleared. Overrides `password` if set. |
| `tailscale-authkey` | string | — | Tailscale authentication key (e.g. `"tskey-auth-xxxxx"`). If tailscale netcheck passed during installation, the installer attempts to authenticate directly by chrooting into the target system. If that doesn't run or fails, the key is written to `/etc/bes/tailscale-authkey` for first-boot authentication. |
| `ssh-authorized-keys` | array of strings | `[]` | SSH public keys to install for the default user. Each entry must be a non-empty SSH public key string (e.g. `"ssh-ed25519 AAAA... admin@example.com"`). |
| `timezone` | string | `"UTC"` | IANA timezone for the installed system (e.g. `"Pacific/Auckland"`, `"America/New_York"`). The installer creates a symlink at `/etc/localtime` pointing to `/usr/share/zoneinfo/<timezone>` and writes the name to `/etc/timezone`. In the TUI, a searchable list of timezones is presented; type to filter and use Up/Down to navigate. |
