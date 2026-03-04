# Plan: DHCP Hostname Support for Metal Variant

## Problem

When the metal disk image is written directly to a disk (bypassing the
installer), the system boots with a static hostname of `ubuntu` baked into
`/etc/hostname`. Unlike the cloud variant, there is no cloud-init to override
this from instance metadata. The machine is stuck with a generic hostname
unless the user manually changes it.

The `systemd-networkd` DHCP client already supports receiving a hostname from
the DHCP server and setting it as the transient hostname via
`systemd-hostnamed`. However, `systemd-hostnamed` only applies a
DHCP-provided transient hostname when the static hostname is unset (i.e.
`/etc/hostname` is empty or missing). Since the image currently writes
`ubuntu` to `/etc/hostname`, DHCP hostnames are silently ignored.

## Goals

1. Metal images written directly to disk get their hostname from DHCP
   automatically (when the DHCP server provides one).
2. The installer gives the user an explicit choice: set a static hostname
   (default) or opt into DHCP dynamic hostname assignment.
3. The cloud image is unaffected (cloud-init handles hostname).
4. The `bes-install.toml` config file supports the new option for auto mode.

## Design

### Image build (`image/configure.sh`)

The hostname section currently does:

```sh
echo "ubuntu" > /etc/hostname
```

Change to be variant-aware:

- **Metal**: write an empty `/etc/hostname` (truncate to zero bytes). This
  makes `systemd-hostnamed` show `Static hostname: n/a` and accept transient
  hostnames from DHCP. `/etc/hosts` keeps only `localhost` entries (no
  `127.0.1.1` line, since there is no static hostname to map).
- **Cloud**: keep writing `ubuntu` to `/etc/hostname`. Cloud-init with
  `create_hostname_file: false` already prevents cloud-init from touching the
  file; the hostname comes from DHCP or instance metadata at runtime, and
  `ubuntu` serves as a sensible fallback when neither provides one.

A new spec rule `r[image.hostname.metal-dhcp]` captures this: the metal image
must ship with an empty `/etc/hostname` so that `systemd-hostnamed` accepts
DHCP-provided transient hostnames. A corresponding rule
`r[image.hostname.cloud-default]` captures the cloud behavior: the cloud
image ships with `ubuntu` as the static hostname.

### Installer config schema

Add a new field to the `[firstboot]` table:

```toml
[firstboot]
# Use DHCP-provided hostname instead of a static one.
# Mutually exclusive with hostname.
hostname-from-dhcp = true
```

- `hostname` and `hostname-from-dhcp` are mutually exclusive. If both are
  present, the installer reports a validation error (like `password` and
  `password-hash`).
- For auto mode with the metal variant, the requirement changes: either
  `hostname` or `hostname-from-dhcp = true` must be present (at least one
  hostname strategy must be explicitly chosen). This replaces the current
  "hostname is required for metal" rule.
- For the cloud variant, both fields remain fully optional.

### Installer TUI

The current Hostname screen has a text input and blocks advancing with an
empty field for the metal variant.

Redesign for metal:

- The screen shows the text input for hostname (as today).
- Below the text input, a toggle/checkbox: `[ ] Use DHCP hostname (no static
  hostname)`.
- The toggle is activated with a keybind (e.g. Tab to switch focus between
  the text input and the toggle, Space to toggle).
- When the toggle is ON: the text input is visually dimmed/disabled, and the
  screen displays a note: "The system will get its hostname from DHCP. The
  static hostname will be empty (shown as n/a by hostnamectl)."
- When the toggle is OFF (default): the hostname text input is active and
  must be non-empty to advance (current metal behavior).
- Advancing is allowed when either (a) the toggle is ON, or (b) a non-empty
  hostname is entered.
- If the config file has `hostname-from-dhcp = true`, the toggle is
  pre-activated and the text input is empty.

For cloud:

- The toggle is not shown. The text input remains optional as today. The
  hint says "Leave empty to skip (default: ubuntu, overridden by
  DHCP/cloud-init)."

### Installer firstboot logic (`firstboot.rs`)

`apply_hostname` currently writes to `/etc/hostname` and appends to
`/etc/hosts`. The new logic:

- **`hostname` is set**: write the hostname to `/etc/hostname`, add
  `127.0.1.1 <hostname>` to `/etc/hosts`. (No change from today.)
- **`hostname-from-dhcp` is true**: write an empty file to `/etc/hostname`
  (truncate). Remove any `127.0.1.1` line from `/etc/hosts` if present.
  This restores the metal image's default state, which is useful when the
  installer is used on the cloud image (the cloud image ships with `ubuntu`
  in `/etc/hostname`, and the user explicitly wants DHCP).
- **Neither set (cloud only)**: leave `/etc/hostname` as-is in the written
  image. For cloud, this means `ubuntu` persists. For metal, this means
  the empty file from the image persists. (The installer already requires
  one of the two for metal, so this case only occurs for cloud.)

### `FirstbootConfig` struct

```rust
pub struct FirstbootConfig {
    pub hostname: Option<String>,
    pub hostname_from_dhcp: bool,  // new field, default false
    pub tailscale_authkey: Option<String>,
    pub ssh_authorized_keys: Vec<String>,
    pub password: Option<String>,
    pub password_hash: Option<String>,
}
```

`has_hostname_config()` returns true if `hostname.is_some() ||
hostname_from_dhcp`.

### Dry-run schema

The `firstboot.hostname` field in the install plan JSON changes:

- When a static hostname is set: `"hostname": "server-01"` (string, as
  today).
- When DHCP hostname is chosen: `"hostname": "dhcp"` (the string `"dhcp"`
  as a sentinel).
- When neither is set: `"hostname": null` (as today, cloud-only case).

### Validation

In `InstallConfig::validate()`:

- If both `hostname` and `hostname-from-dhcp` are set, emit an error:
  "firstboot.hostname and firstboot.hostname-from-dhcp are mutually
  exclusive".
- Existing hostname format validation (length, characters) still applies
  when `hostname` is set.
- `hostname-from-dhcp` on cloud emits a warning: "hostname-from-dhcp has
  no special effect with the cloud variant (DHCP hostname is already the
  default)".

In `mode()`:

- For metal auto, the completeness check becomes: `hostname.is_some() ||
  hostname_from_dhcp` (instead of just `hostname.is_some()`).

### AppState / TUI state

New fields:

```rust
pub hostname_from_dhcp: bool,  // toggle state, default false
```

`hostname_required()` becomes: `self.variant == Variant::Metal &&
!self.hostname_from_dhcp`.

`firstboot_config()` builds the config with:
- `hostname_from_dhcp: self.hostname_from_dhcp`
- `hostname`: from the text input if non-empty (and DHCP toggle is off)

### Container test scenarios

Update `test-container-install-all.sh`:

- Add a new scenario: metal with `hostname-from-dhcp = true` and no
  `hostname`. Verify that `/etc/hostname` is empty on the written disk.
- Update existing metal scenarios that now set hostname to ensure they still
  work.

Update `test-container-install.sh`:

- Add a `SET_HOSTNAME_FROM_DHCP` env var.
- When set, add `hostname-from-dhcp = true` to the generated TOML.
- In the verification section, when `SET_HOSTNAME_FROM_DHCP` is set, check
  that `/etc/hostname` exists and is empty (zero bytes).

### Image structure tests

Update `test-image-structure.sh`:

- For metal images: verify `/etc/hostname` is empty.
- For cloud images: verify `/etc/hostname` contains `ubuntu`.

### README

Update the Hostname section:

- Metal direct-write: the image ships with an empty `/etc/hostname`, so the
  system automatically picks up a DHCP-provided hostname. If DHCP does not
  provide one, `hostnamectl` shows `Static hostname: n/a` and the transient
  hostname is `localhost`.
- Metal via installer: the user chooses between a static hostname (default)
  or DHCP hostname. Describe the toggle.
- Cloud: unchanged (default `ubuntu`, overridden by cloud-init/DHCP).

Update the field reference table:

- Add `hostname-from-dhcp` row to the `[firstboot]` table.
- Note the mutual exclusivity with `hostname`.

## File change summary

| File | Change |
|------|--------|
| `docs/spec/linux-images.md` | Add `image.hostname.metal-dhcp` and `image.hostname.cloud-default` rules. Update `installer.config.schema`, `installer.tui.hostname`, `installer.mode.auto`, `installer.mode.auto-incomplete`, `installer.firstboot.hostname`, `installer.dryrun.schema`. |
| `image/configure.sh` | Make hostname section variant-aware (empty for metal, `ubuntu` for cloud). |
| `installer/tui/src/config.rs` | Add `hostname_from_dhcp` to `FirstbootConfig`. Update `validate()` for mutual exclusivity and cloud warning. Update `mode()` completeness check. |
| `installer/tui/src/ui.rs` | Add `hostname_from_dhcp` to `AppState`. Update `hostname_required()`, `firstboot_config()`, `advance()` logic. |
| `installer/tui/src/ui/run.rs` | Handle Tab (focus switch) and Space (toggle) on Hostname screen for metal. |
| `installer/tui/src/ui/render.rs` | Render the DHCP toggle for metal. Dim the text input when toggle is active. Show explanatory note. |
| `installer/tui/src/firstboot.rs` | Handle `hostname_from_dhcp`: truncate `/etc/hostname`, clean `127.0.1.1` from `/etc/hosts`. |
| `installer/tui/src/plan.rs` | Emit `"hostname": "dhcp"` when `hostname_from_dhcp` is true. |
| `installer/tui/src/main.rs` | Update auto-incomplete message (hostname *or* hostname-from-dhcp required for metal). |
| `installer/tui/tests/dry_run.rs` | Add tests for `hostname-from-dhcp` in auto and interactive modes. Update existing tests as needed. |
| `tests/test-container-install.sh` | Add `SET_HOSTNAME_FROM_DHCP` env var, TOML generation, and verification. |
| `tests/test-container-install-all.sh` | Add metal DHCP hostname scenario. |
| `tests/test-image-structure.sh` | Verify `/etc/hostname` is empty for metal, `ubuntu` for cloud. |
| `README.md` | Update Hostname section and field reference table. |

## Implementation order

1. Spec updates (all rule changes).
2. Image build change (variant-aware `/etc/hostname`).
3. Image structure test update.
4. Config schema + validation changes.
5. Firstboot logic (apply DHCP hostname).
6. TUI state + render + key handling.
7. Dry-run plan output.
8. Unit tests for all of the above.
9. Dry-run integration tests.
10. Container test scenarios.
11. README.
12. Final `cargo clippy`, `cargo fmt`, `tracey query status`.