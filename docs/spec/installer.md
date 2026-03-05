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
> # IANA timezone (e.g. "America/New_York"). Defaults to "UTC".
> timezone = "Pacific/Auckland"
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

r[installer.mode.auto.progress]
In automatic mode, the installer must detect whether standard error is
connected to a terminal. When it is a terminal (interactive), progress
updates use a carriage return (`\r`) to overwrite the current line.
When it is not a terminal (e.g. CI log output), the installer must
suppress the per-update progress lines entirely and instead print a
single summary line after the write completes (total bytes written,
throughput, and elapsed time). This avoids flooding non-interactive
logs with thousands of nearly-identical progress lines.

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
>     "password_set": true,
>     "timezone": "UTC"
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
> it is `false`. `timezone` is always present in the `firstboot` object and
> defaults to `"UTC"`.

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

r[installer.tui.confirmation+3]
After the timezone screen, and after the pre-summary network results screen,
the TUI must show a summary screen listing: target disk (path, model, size),
chosen variant, TPM enrollment status, and any first-boot configuration. The
summary must clearly state that all data on the target disk will be
destroyed. The user must type an explicit confirmation
(not just press Enter). The confirmation screen is step 6/6.

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

r[installer.firstboot.timezone]
The installer must set the system timezone on the installed system by
creating a symlink at `/etc/localtime` pointing to the corresponding
file under `/usr/share/zoneinfo/` and writing the timezone name to
`/etc/timezone`. The default timezone is `UTC`.

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
