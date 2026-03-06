# Installer

## Configuration File

> r[installer.config.location]
> The installer must look for a TOML configuration file named
> `bes-install.toml`. It searches the following locations in order, using
> the first file found:
>
> 1. The BESCONF partition (mounted at `/run/besconf/` by a udev rule or
>    mount unit in the live environment).
> 2. `/run/live/medium/bes-install.toml` (the ISO filesystem root, as
>    mounted by `live-boot`).
> 3. `/boot/efi/bes-install.toml` (fallback for manual placement).

r[installer.config.format]
The configuration file is in TOML format. All fields are top-level
(no tables or sections) and all fields are optional. The installer must
reject unknown fields with a parse error. The file configures both the
installation process (disk selection, encryption, automation) and the
install-time system setup (hostname, credentials, services).

r[installer.config.auto]
The `auto` field is a boolean. When `true`, the installer runs fully
automatically without interactive prompts. Automatic mode requires at
minimum `disk-encryption` and `disk` to be set; if they are missing the
installer must report a validation error.

r[installer.config.disk-encryption]
The `disk-encryption` field is a string selecting the disk encryption
mode. Valid values are `"tpm"` (LUKS + TPM PCR 1; requires a TPM and is
the default when a TPM is present), `"keyfile"` (LUKS + keyfile on the
boot partition; default when no TPM is present), or `"none"` (no
encryption, producing a cloud image).

r[installer.config.disk]
The `disk` field is a string selecting the target disk. It may be a
device path (e.g. `"/dev/sda"`) or a strategy name. Valid strategies are
`"largest-ssd"` (the largest SSD/NVMe device), `"largest"` (the largest
device regardless of transport), and `"smallest"` (the smallest device).

r[installer.config.copy-install-log]
The `copy-install-log` field is a boolean controlling whether the
installer copies its log into the installed system at
`/var/log/bes-installer.log`. The default is `true`. There is no TUI
control for this option.

r[installer.config.hostname]
The `hostname` field is a string setting a static hostname for the
installed system (e.g. `"server-01"`). The `hostname-from-dhcp` field is
a boolean; when `true`, the installed system obtains its hostname from
DHCP. The `hostname-template` field is a string that generates a hostname
from a template pattern (see `installer.config.hostname-template`). These
three fields are mutually exclusive; if more than one is present the
installer must report a validation error.

r[installer.config.tailscale-authkey+2]
The `tailscale-authkey` field is a string containing a Tailscale auth key
(e.g. `"tskey-auth-xxxxx"`). When set, the installer uses it to
authenticate the installed system with Tailscale (see
`r[installer.finalise.tailscale-authkey]` for the authentication procedure).

r[installer.config.ssh-authorized-keys+2]
The `ssh-authorized-keys` field is an array of strings, each an OpenSSH
public key line (e.g. `"ssh-ed25519 AAAA... admin@example.com"`). When
set, the installer writes these keys into the installed system (see
`r[installer.finalise.ssh-keys]`).

r[installer.config.password]
The `password` field is a string containing a plaintext password for the
`ubuntu` user. The `password-hash` field is a string containing a
pre-hashed password in crypt(3) format (e.g. `"$6$rounds=4096$..."`).
These two fields are mutually exclusive; if both are present the installer
must report a validation error.

r[installer.config.timezone]
The `timezone` field is a string containing an IANA timezone name (e.g.
`"Pacific/Auckland"`). The default is `"UTC"`.

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

r[installer.mode.interactive+2]
When no configuration file is found, the installer must launch a fully
interactive TUI with sensible defaults (disk encryption auto-detected based
on TPM presence, disk strategy `largest-ssd`).

r[installer.mode.prefilled]
When a configuration file is present but `auto` is false or absent, the
installer must launch the TUI with values from the file pre-filled as
defaults. The user can override any value.

> r[installer.mode.auto+4]
> When `auto = true` and all required fields are present, the installer must
> proceed without any interactive prompts. It must:
>
> 1. Log its configuration to the console.
> 2. Display progress during image writing.
> 3. Apply install-time configuration.
> 4. Apply encryption setup (key rotation, TPM/keyfile enrollment, recovery
>    passphrase) when disk encryption is not `"none"`.
> 5. Reboot automatically on success.
> 6. Print an error and exit with a non-zero status on failure.
>
> Required fields: `disk-encryption`, `disk`. Additionally, when
> `disk-encryption` is `"tpm"` or `"keyfile"`, at least one hostname
> strategy must be specified: `hostname`,
> `hostname-from-dhcp = true`, or `hostname-template`.

r[installer.mode.auto.progress]
In automatic mode, the installer must detect whether standard error is
connected to a terminal. When it is a terminal (interactive), progress
updates use a carriage return (`\r`) to overwrite the current line.
When it is not a terminal (e.g. CI log output), the installer must
suppress the per-update progress lines entirely and instead print a
single summary line after the write completes (total bytes written,
throughput, and elapsed time). This avoids flooding non-interactive
logs with thousands of nearly-identical progress lines.

r[installer.mode.auto-incomplete+3]
When `auto = true` but required fields are missing (`disk-encryption`,
`disk`, or a hostname strategy for encrypted variants), the installer
must print an error describing the missing fields and fall back to
interactive mode.

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

> r[installer.dryrun.schema+5]
> The install plan JSON has the following structure:
>
> ```json
> {
>   "mode": "auto | prefilled | interactive | auto-incomplete",
>   "disk_encryption": "tpm | keyfile | none",
>   "variant": "metal | cloud",
>   "disk": {
>     "path": "/dev/nvme0n1",
>     "model": "Samsung 980 PRO",
>     "size_bytes": 1000204886016,
>     "transport": "NVMe"
>   },
>   "tpm_present": true,
>   "install_config": {
>     "hostname": "server-01",
>     "hostname_from_template": false,
>     "tailscale_authkey": true,
>     "ssh_authorized_keys_count": 2,
>     "password_set": true,
>     "timezone": "UTC"
>   },
>   "manifest_path": "/run/live/medium/images/partitions.json",
>   "copy_install_log": true,
>   "config_warnings": []
> }
> ```
>
> `disk_encryption` is the user's chosen encryption mode. `variant` is a
> derived field: `"metal"` when `disk_encryption` is `"tpm"` or `"keyfile"`,
> `"cloud"` when `"none"`. `tpm_present` indicates whether a TPM was
> detected (or faked via `--fake-tpm`).
>
> The `install_config` field is `null` when no install-time configuration
> fields (hostname, tailscale, SSH keys, password, or timezone) are set.
> `tailscale_authkey` is a boolean (true when a key is provided) to avoid
> leaking secrets into test output. `password_set` is a boolean (true when
> a password or password hash is provided). When `hostname-from-dhcp` is
> chosen, `hostname` is the sentinel string `"dhcp"`. When a hostname was
> generated from a template, `hostname_from_template` is `true`; otherwise
> it is `false`. `timezone` is always present in the `install_config`
> object and defaults to `"UTC"`.

r[installer.dryrun.devices]
In dry-run mode the installer must still detect real block devices (via
`lsblk`) unless a `--fake-devices <path>` flag is given, in which case
it reads device definitions from a JSON file instead. The JSON file must
be an array of objects with the same fields as the `disk` object in the
install plan (`path`, `model`, `size_bytes`, `transport`), plus an
optional `removable` boolean (default false).

r[installer.dryrun.fake-tpm]
The `--fake-tpm` flag forces the installer to behave as if a TPM device is
present, regardless of whether `/dev/tpm0` exists. This is used for testing
the TPM-bound encryption path in environments without a real TPM.

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

r[installer.tui.welcome+3]
The TUI must open with a welcome screen that displays a description of what
the image is for, contact information, and instructions on how to proceed.
The user presses Enter to proceed to the disk selection screen. The welcome
screen also offers a `n` keybind to open a dedicated network check screen.

> r[installer.tui.network-check+4]
> The TUI must perform network connectivity checks in the background,
> starting automatically when the welcome screen is first shown. The checks
> run against the following endpoints in parallel:
>
> - `https://ghcr.io/` — expects HTTP 200
> - `https://meta.tamanu.app/` — expects HTTP 200
> - `https://tools.ops.tamanu.io/` — any HTTP response (even 403) is a pass
> - `https://clients.ops.tamanu.io/` — any HTTP response is a pass
> - `https://servers.ops.tamanu.io/` — any HTTP response is a pass
> - `https://github.com/` — any HTTP response is a pass
> - An NTP server (`pool.ntp.org`) over UDP port 123 — a UDP socket connect succeeds
>
> Each check has a 5-second timeout. Results are displayed as a list with a
> pass/fail indicator next to each endpoint. Failures are not blocking.
>
> The network check results are presented in two places:
>
> 1. **Dedicated network check screen** — accessible from the welcome screen
>    via the `n` keybind. This screen first checks whether any network
>    interface has connectivity. If there is no network at all, a message is
>    shown to the user. Otherwise the individual endpoint check results are
>    displayed live as they complete, followed by the output of
>    `tailscale netcheck`. The user can press `r` to re-run all checks, and
>    `Esc` to return to the welcome screen.
>
> 2. **Pre-summary network results screen** — shown between the timezone
>    screen and the confirmation screen. This screen displays the results of
>    the background checks (which were started on the welcome screen and have
>    likely completed by now). If the checks have not yet finished, the
>    screen shows progress. The user can press `r` to re-run all checks, and
>    `Enter` to proceed to the confirmation screen.
>
> Both screens are skipped entirely in automatic mode.
>
> The two network panes (Connectivity and Tailscale Netcheck) are displayed
> as an accordion: the active pane is expanded and the inactive pane is
> collapsed to a title bar. Both panes use the same border color (normal
> text, not dimmed or accented) so the active/inactive distinction comes
> from the expanded-vs-collapsed layout alone. Each pane's title bar
> includes a status indicator: a spinner or "Running..." while in progress,
> "All passed" or "N/M passed" when done for connectivity, and "OK" or
> "Failed" when done for tailscale netcheck. There is no separate summary
> line outside the panes.

r[installer.tui.tailscale-netcheck+2]
The TUI must run `tailscale netcheck` in the background (the `tailscale`
binary must be available on the live ISO). The check starts automatically
alongside the network connectivity checks when the welcome screen is first
shown. If the `tailscale` binary is not found or the command fails, the
result stores an appropriate error message. The tailscale netcheck output is
displayed on both the dedicated network check screen and the pre-summary
network results screen, below the endpoint check results.

r[installer.tui.disk-detection+3]
After the welcome screen (or automatically in automatic mode), the TUI must
detect available block devices and display their device path, size, model
name, and transport type (SSD, HDD, NVMe, USB, etc.).

> r[installer.tui.disk-encryption+2]
> After the disk selection screen, the TUI must present a "Disk Encryption"
> screen. The installer detects whether a TPM is present by checking for
> `/dev/tpm0` (or via the `--fake-tpm` flag). The screen displays a radio
> selection with contextual explanation text below it.
>
> If a TPM is present, three options are shown:
>
> - **Full-disk encryption, bound to hardware** (default)
> - **Full-disk encryption, not bound to hardware**
> - **No encryption**
>
> If no TPM is present, two options are shown:
>
> - **Full-disk encryption, not bound to hardware** (default)
> - **No encryption**
>
> Explanation text changes based on the current selection:
>
> - **Bound to hardware**: "The disk's encryption key will be sealed to this
>   machine's TPM using PCR 1 (hardware identity: motherboard, CPU, and RAM
>   model/serials). The system will boot unattended as long as the hardware
>   stays the same. If you move the disk to different hardware, you will need
>   the recovery passphrase. Changing the CPU or RAM may also require the
>   recovery passphrase."
> - **Not bound to hardware**: "A keyfile will be stored on the boot
>   partition. The system will boot unattended on any hardware. If the boot
>   partition is lost, you will need the recovery passphrase."
> - **No encryption**: "The root partition will not be encrypted."



> r[installer.tui.hostname+5]
> After disk encryption selection, the TUI presents a hostname selection
> screen. The screen offers two options via an Up/Down selector:
>
> - **Static hostname**
> - **Network-assigned (DHCP)** (metal variant) or **Network-assigned
>   (DHCP / cloud-init)** (cloud variant)
>
> When encryption is selected, "Static hostname" is selected by default.
> Otherwise, the network-assigned option is selected by default.
>
> Enter confirms the selection. If "Static hostname" is chosen, a second
> sub-screen (`HostnameInput`) presents a text input for the hostname. The
> field may be pre-filled from the configuration file or a resolved hostname
> template. The hostname is required: the user must enter a non-empty value
> to advance, and an inline error is shown if the field is empty on Enter.
> This applies to both variants — choosing "Static hostname" is an explicit
> decision to set a hostname, so an empty value is never accepted.
>
> The hostname is also validated: it must contain only ASCII
> letters, digits, and hyphens (`a-z`, `0-9`, `-`), must not start or end
> with a hyphen, and must not exceed 63 characters. If validation fails, an
> inline error describing the problem is shown and advance is blocked.
> Esc from the text input returns to the selection screen.
>
> If the network-assigned option is chosen, the TUI advances directly to
> the Login screen with `hostname_from_dhcp` set to true and no text input
> step. Esc from the selection screen returns to the previous screen
> (TpmToggle for metal, VariantSelection for cloud).
>
> When a `hostname-template` is present in the configuration, the template
> is resolved to a concrete hostname at startup, pre-fills the text input,
> and the selector defaults to "Static hostname" regardless of variant.

r[installer.tui.tailscale+3]
After the hostname screen, the TUI presents a Login screen. The Login screen
has inline password entry and keybinds to open sub-screens for Tailscale
auth key, SSH keys, and GitHub SSH key import. The Tailscale sub-screen is
accessed via the `Alt+t` keybind from the Login screen and presents a text input
for a Tailscale auth key. The field may be pre-filled from the configuration
file. The user can leave it empty to skip Tailscale configuration. Enter or
Esc returns to the Login screen.

r[installer.tui.ssh-keys+5]
The SSH keys sub-screen is accessed via the `Alt+s` keybind from the Login
screen. It displays a list of individual key entry fields with a trailing
blank field always present at the end. The selected field is expanded for
editing as a bordered text input; non-selected fields are collapsed to a
one-line summary showing the key type, start of the key material, and the
comment (if any). Empty entries are shown as `(empty)` in gray. Non-empty
entries that do not pass the validity check are highlighted in red (both
when selected for editing and when collapsed) to indicate they will be
discarded on exit. When the
user types into the trailing blank field, a new blank field is automatically
appended so there is always a blank field at the end. Tab cycles forward
through the fields (wrapping from the last to the first). Shift+Tab cycles
backward (wrapping from the first to the last). Enter or Esc returns to the
Login screen after filtering out empty and invalid entries. A minimal
validity check requires the line to start with a recognized key type prefix
(`ssh-rsa`, `ssh-ed25519`, `ssh-dss`, `ecdsa-sha2-nistp256`,
`ecdsa-sha2-nistp384`, `ecdsa-sha2-nistp521`,
`sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com`)
followed by a space and at least one more non-whitespace character. After
filtering, if the vec is empty, a single empty string is re-added so the
screen always has at least one field.

r[installer.tui.ssh-keys.github+4]
The GitHub import sub-screen is accessed via the `Alt+g` keybind from the Login
screen, only when `github.com` is reachable per the background network
checks. It presents a text input for a GitHub username. When the user
presses Enter, the installer fetches `https://github.com/<username>.keys`.
If the fetch succeeds and returns one or more keys, they are appended as
individual entries in the SSH keys list, then the screen navigates to the
SSH Keys sub-screen so the user can review the imported keys.
If the fetch fails or returns no keys, an inline error is displayed. The
fetch must time out after 5 seconds. Esc returns to the Login screen.

r[installer.tui.password+4]
Password entry is inline on the Login screen. The user types a password,
then confirms it by typing it again. Both fields are masked (displayed as
asterisks). If the two entries do not match, the TUI must display an inline
error and not advance. The password must not be empty in interactive mode;
if the user attempts to advance with both fields empty, the TUI must display
an inline error ("Password is required") and not advance. When a password is
provided via the configuration file (`password` or `password-hash`), the
password fields are pre-satisfied and the screen is skipped in prefilled and
auto modes. Below the password fields, the Login
screen shows keybind hints for the sub-screens (`Alt+t`: Tailscale, `Alt+s`:
SSH keys, `Alt+g`: GitHub import). The Alt modifier prevents the keybinds
from interfering with password input. A yellow `*` indicator is appended to
each hint when a value is set (Tailscale auth key is non-empty, or SSH keys
are present). The `Alt+g` hint is only shown when `github.com` is reachable.

r[installer.tui.timezone]
After the password screen, the TUI must present a timezone selection screen.
The screen displays a searchable list of IANA timezones read from the
system's `/usr/share/zoneinfo` (specifically by parsing
`/usr/share/zoneinfo/zone1970.tab`). The user types to filter the list and
uses Up/Down arrows to navigate the filtered results. Enter selects the
highlighted timezone and advances to the next screen. The field defaults to
`UTC` and may be pre-filled from the configuration file. If a
`--fake-timezones <path>` flag is given, the installer reads timezone names
(one per line) from that file instead of the system tzdata.

r[installer.tui.confirmation+7]
After the timezone screen, and after the pre-summary network results screen,
the TUI must show a summary screen listing: target disk (path, model, size),
chosen disk encryption mode, and any install-time configuration. The summary
must clearly state that all data on the target disk will be destroyed. The
user must type an explicit confirmation (not just press Enter). The
confirmation screen is step 6/6.

When disk encryption is enabled (`"tpm"` or `"keyfile"`), the confirmation
screen must also generate and display the recovery passphrase. This gives
the user an opportunity to write it down **before** the destructive write
begins. The same passphrase is later enrolled into the LUKS volume during
encryption setup. The completion screen displays the recovery passphrase
again as a final reminder; the user must press Enter to acknowledge before
the system reboots.

r[installer.tui.ascii-rendering]
All text rendered by the TUI must use only printable ASCII characters. In
particular, em dashes, curly quotes, ellipsis characters, and other
non-ASCII punctuation must be replaced with their ASCII equivalents (e.g.
`--` instead of U+2014). The Linux console (the default terminal on bare
metal) does not have Unicode fonts; non-ASCII characters render as
replacement blocks.

r[installer.tui.error-reboot]
When the installer encounters a fatal error during the write or post-write
phases, the TUI must display the error message and wait for a keypress.
Pressing any key must trigger a reboot (or exit cleanly if `--no-reboot` is
set), not simply quit the process. On bare-metal hardware, quitting without
rebooting leaves the machine in an unusable state.

r[installer.tui.progress+3]
The TUI must display a single progress bar that covers the entire
installation, not just the image write. The progress bar is shown on one
`Installing` screen from the moment the user confirms until all steps
complete. Partition image writes (which have byte-level progress) occupy
approximately 90% of the bar. Each post-write step (filesystem expansion,
UUID randomization, boot config rebuild, partition verification, install-time
configuration, and encryption setup) occupies a small fixed slice of the
remaining 10%, advancing the bar when the step completes. After all steps
finish, the TUI transitions to a completion screen. For encrypted installs,
the completion screen also displays the recovery passphrase (replacing the
separate recovery passphrase screen).

r[installer.tui.debug-shell]
Pressing `Ctrl+Alt+d` at any point in the TUI must drop the user into an
interactive shell (`/bin/sh`). The TUI must leave the alternate screen,
disable raw mode, and spawn the shell as a child process, waiting for it to
exit. When the shell exits, the TUI must re-enter the alternate screen,
re-enable raw mode, and redraw. This keybind is intentionally undocumented
in the on-screen help; it is a debugging aid for diagnosing failures in
container and bare-metal environments.

r[installer.tui.loop-device]
The installer's TUI and write pipeline must not assume the target device is
real hardware. It must work correctly when targeting a loop device backed by
a sparse file (created via `losetup --partscan`). This means no reliance on
udev events for partition discovery (explicit `partprobe` calls are
acceptable), no transport-type filtering that would reject loop devices, and
no SCSI/ATA-specific ioctls.

## Image Writing

r[installer.write.partitions+2]
Before writing partition images, the installer must wipe all existing
filesystem, RAID, and partition-table signatures from the target disk. It
must then create the GPT table and all three partitions (EFI, xboot, root)
using the geometry from `partitions.json`. After writing all partition
images, the installer must verify the partition table.

r[installer.write.source+2]
The installer must read `partitions.json` from the ISO filesystem to locate
the partition images and their layout metadata. There is one set of partition
images per architecture, not per variant. The installer must search the
standard ISO mount paths (`/run/live/medium/images`, `/cdrom/images`, etc.)
for the manifest file.

r[installer.write.disk-size-check+2]
Before writing, the installer must read the uncompressed size of each
partition image from its `.size` sidecar file and verify that the target disk
is at least as large as the sum of all partition sizes (plus GPT overhead).
If the disk is too small, the installer must refuse to write and report the
required size and disk size in the error message.

r[installer.write.decompress-stream+2]
The installer must stream-decompress each zstd-compressed partition image
directly to its corresponding partition device (or to the opened LUKS mapper
device for the root partition when encryption is enabled), avoiding the need
to hold the uncompressed image in memory or on a temporary filesystem.

r[installer.write.luks-before-write+2]
When disk encryption is not `"none"`, the installer must format the root
partition with LUKS2 using the recovery passphrase as the initial key and
open the LUKS volume before writing the root partition image. After writing,
the LUKS volume must be closed. The recovery passphrase must be generated
before the write begins (in interactive mode it is generated at confirmation
time; in automatic mode it is generated before the write phase). Since the
installer creates the LUKS volume itself, there is no need for a key
rotation step or an empty-passphrase slot. The same recovery passphrase is
used to unlock the volume during subsequent post-write steps (expand root,
randomize UUIDs, rebuild boot config).

r[installer.write.fstab-fixup]
When disk encryption is not `"none"`, the installer must rewrite `/etc/fstab`
on the installed system to reference `/dev/mapper/root` instead of
`/dev/disk/by-partlabel/root` for the root and postgresql mount entries.

r[installer.write.variant-fixup]
The installer must write the correct variant name to `/etc/bes/image-variant`
on the installed system: `metal` when disk encryption is not `"none"`, `cloud`
when disk encryption is `"none"`.

r[installer.write.expand-root]
After writing the root partition image, the installer must expand the root
filesystem to fill the partition. When disk encryption is enabled, the
installer must first open the LUKS volume and run `cryptsetup resize` to
expand it to fill the partition, then run `btrfs filesystem resize max` on
the mounted BTRFS. When encryption is `"none"`, only the BTRFS resize is
needed. This ensures the installed system has a fully expanded filesystem
without depending on a boot-time growth service.

r[installer.write.randomize-uuids+2]
After expanding the root filesystem, the installer must randomize the
filesystem UUID of each partition to ensure every installation has unique
identifiers. For the ext4 extended boot partition, it must first run
`e2fsck -f -y` (required by `tune2fs` before modifying the superblock),
then `tune2fs -U random`. For the BTRFS root partition (or the LUKS volume
on top of it), it must run `btrfstune -u` while the filesystem is
unmounted. For the FAT32 EFI partition, it must randomize the volume serial
number with `mlabel -n`. All filesystems must be unmounted during UUID
changes.

r[installer.write.rebuild-boot-config]
After randomizing filesystem UUIDs, the installer must unconditionally
rebuild the initramfs and GRUB configuration in a chroot of the installed
system, regardless of encryption mode. This is required because the GRUB
config (`grub.cfg`) and the initramfs both reference filesystem UUIDs that
have been rotated. The installer must run `dracut --force` and `update-grub`
with `/proc`, `/sys`, and `/dev` bind-mounted into the target.

## Encryption Setup

> r[installer.encryption.overview+2]
> After writing the image and expanding partitions, and when disk encryption
> is `"tpm"` or `"keyfile"`, the installer must perform all encryption setup
> on the target disk. The LUKS volume already has the recovery passphrase as
> its sole key (enrolled during `installer.write.luks-before-write`). The
> installer must:
>
> 1. Enroll the chosen unlock mechanism (TPM or keyfile).
> 2. Configure the installed system (crypttab, initramfs).
>
> No key rotation or empty-slot wipe is needed because the installer created
> the LUKS volume with fresh key material and the recovery passphrase as the
> initial key.



> r[installer.encryption.tpm-enroll+2]
> When disk encryption is `"tpm"`, the installer must enroll the TPM using
> `systemd-cryptenroll` with `--tpm2-pcrs=1`, unlocking the volume with the
> recovery passphrase. PCR 1 covers hardware identity (motherboard model,
> CPU, RAM model and serials). The installer must update `/etc/crypttab` to
> use `tpm2-device=auto` with a passphrase timeout fallback.

> r[installer.encryption.keyfile-enroll+2]
> When disk encryption is `"keyfile"`, the installer must generate a random
> keyfile (4096 bytes from `/dev/urandom`), enroll it via
> `cryptsetup luksAddKey` unlocking with the recovery passphrase, and
> install it at `/etc/luks/keyfile` (mode 000) on the installed system. The
> installer must update `/etc/crypttab` to reference the keyfile with a
> passphrase timeout fallback, and update the dracut configuration to
> include the new keyfile in the initramfs.

> r[installer.encryption.recovery-passphrase+3]
> The installer must generate a human-readable recovery passphrase before the
> write phase begins. This passphrase is used as the initial LUKS key when
> formatting the root partition (see `r[installer.write.luks-before-write]`),
> so it is already enrolled as a LUKS password slot — no separate
> `luksAddKey` step is required. In interactive mode, the passphrase must be
> generated at confirmation time and displayed on the confirmation screen so
> the user can record it. A post-write "Recovery Passphrase" screen is shown
> as a reminder before the "Done" screen; the user must press Enter to
> acknowledge. In automatic mode, the passphrase is generated before the
> write phase and printed to stderr after install completes.



> r[installer.encryption.configure-system]
> The installer must chroot into the installed system and rebuild the
> initramfs with dracut so it picks up the updated crypttab and (if keyfile
> mode) the new keyfile.

## Install-Time Configuration

r[installer.finalise.mount+4]
After writing the image, the installer must mount the target disk's root
BTRFS partition (subvol `@`) to apply install-time configuration. For the
metal variant (disk encryption `"tpm"` or `"keyfile"`), it must unlock the
LUKS volume using the recovery passphrase.

r[installer.finalise.hostname]
If `hostname` is set (including hostnames generated from a template), the
installer must write it to `/etc/hostname` and add a `127.0.1.1` entry to
`/etc/hosts` on the installed system. If `hostname-from-dhcp` is true, the
installer must write an empty `/etc/hostname` (truncate) and remove any
`127.0.1.1` line from `/etc/hosts`. If neither is set (cloud only), the
installer must leave `/etc/hostname` as-is.

r[installer.finalise.tailscale-authkey+2]
If `tailscale-authkey` is set and the installer has network connectivity,
the installer must attempt to authenticate with Tailscale directly by
chrooting into the mounted target filesystem and running `tailscale up
--auth-key=<key> --ssh`. The installer must display feedback about the
authentication attempt (e.g. success, invalid key, network error). If the
authentication fails, the installer may offer to retry or accept a
different key. Only if authentication does not succeed does the installer
fall back to writing the key to `/etc/bes/tailscale-authkey` for first-boot
authentication via the `r[image.tailscale.firstboot-auth]` systemd service.

r[installer.finalise.ssh-keys]
If `ssh-authorized-keys` is set, the installer must append each key to
`/home/ubuntu/.ssh/authorized_keys` with correct ownership and permissions
(directory 700, file 600, owned by `ubuntu`).

r[installer.finalise.password]
If a password is provided (either plaintext or pre-hashed), the installer
must update the `ubuntu` user's password in `/etc/shadow` on the installed
system. When a plaintext password is given, it must be hashed with SHA-512
crypt (`$6$`). When a pre-hashed password is given, it must be written
directly. In either case, the password expiry flag must be cleared so that
the user is not forced to change the password on first login.

r[installer.finalise.timezone]
The installer must set the system timezone on the installed system by
creating a symlink at `/etc/localtime` pointing to the corresponding
file under `/usr/share/zoneinfo/` and writing the timezone name to
`/etc/timezone`. The default timezone is `UTC`.

r[installer.finalise.copy-install-log+2]
After applying install-time configuration and before encryption setup, the
installer must copy its own log file into the installed system at
`/var/log/bes-installer.log`. This is enabled by default. When
`copy-install-log` is set to `false` in the configuration file, the copy
must be skipped. If the copy fails (e.g. the target filesystem is full or
the source log file does not exist), the installer must log a warning but
must not treat it as a fatal error. There is no TUI control for this
option.

r[installer.finalise.unmount]
After applying configuration, the installer must cleanly unmount all
filesystems and close any LUKS volumes before prompting for reboot.

## Container Isolation

> r[installer.container.isolation+3]
> When the installer is run inside a container (e.g. `systemd-nspawn`) for
> integration testing, it must never have access to the host's real block
> devices. Safety is enforced by three layers:
>
> 1. `systemd-nspawn` provides its own `/dev`; host block devices are not
>    present unless explicitly bound in. Only the loop device itself is
>    bound — partition device nodes are **not** bound from the host.
>    The host `/dev` must **never** be bind-mounted into the container.
> 2. The installer is invoked with `--fake-devices`, which bypasses `lsblk`
>    discovery entirely and presents only the loop device.
> 3. The container runs with `--private-network` by default to prevent any
>    network side-effects. Individual test scenarios may opt out of
>    `--private-network` (e.g. to exercise network-dependent code paths);
>    at least one scenario must run **with** `--private-network` to serve
>    as the enforcement mechanism for `r[iso.offline]`.
>
> The `systemd-nspawn` options and bind-mount configuration used by all
> container scripts (interactive trial, integration tests, isolation test)
> must be defined in a single shared file so that the isolation test
> validates the same container configuration that the installer tests use.
>
> A test must verify this property by launching a container without running
> the installer and confirming that no host block devices (e.g. `/dev/sda`,
> `/dev/nvme*`) are visible inside.

r[installer.container.partition-devices+2]
Inside a container with a private `/dev`, running `partprobe` tells the
kernel to re-read the partition table but the resulting device nodes are
created on the **host's** devtmpfs, not inside the container. The installer
must therefore ensure that partition device nodes exist before any operation
that accesses them (e.g. `cryptsetup open`, `mount`). It does so by reading
`/sys/class/block/<disk>/<partition>/dev` to obtain each partition's
major:minor and then creating or recreating any `/dev` nodes that are
missing or have a stale major:minor (verified via `MetadataExt::rdev()`).
The installer must not attempt to derive partition major:minor numbers from
the parent device — the kernel assigns them dynamically (e.g. loop device
partitions use major 259 with unrelated minors, not `parent_minor + N`).

r[installer.container.swtpm]
Container-based integration tests that exercise TPM disk encryption must use
`swtpm` (software TPM 2.0 emulator) in `chardev` mode with `--vtpm-proxy`
to create a `/dev/tpmN` device on the host. The test harness starts the
`swtpm` process before launching the container and binds the resulting
`/dev/tpmN` device into the container so that `systemd-cryptenroll
--tpm2-device=auto` works against the emulated TPM. The `tpm_vtpm_proxy`
kernel module must be loaded on the host. The `swtpm` process is stopped
and the device cleaned up when the test scenario finishes. The shared
nspawn helpers must support an optional TPM device bind-mount so that only
TPM scenarios pay the setup cost.

r[installer.container.error-logging]
Fatal errors that propagate to the installer's top-level must be logged via
the tracing/log file **in addition to** being printed to stderr, so that
container-based test harnesses that only capture the log file can see the
failure reason.
