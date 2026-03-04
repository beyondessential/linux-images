# Plan: DHCP Hostname Support & Hostname Templates for Metal Variant

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
5. A hostname template mechanism allows auto-generating unique hostnames
   from a pattern, enabling multiple non-interactive installs from a single
   config file without hostname collisions on the network.

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

Add new fields to the `[firstboot]` table:

```toml
[firstboot]
# Use DHCP-provided hostname instead of a static one.
# Mutually exclusive with hostname and hostname-template.
hostname-from-dhcp = true
```

```toml
[firstboot]
# Generate a hostname from a template pattern.
# Mutually exclusive with hostname and hostname-from-dhcp.
hostname-template = "tamanu-{hex:6}"
```

The three hostname fields — `hostname`, `hostname-from-dhcp`, and
`hostname-template` — are mutually exclusive. If more than one is present,
the installer reports a validation error.

- For auto mode with the metal variant, the requirement changes: one of
  `hostname`, `hostname-from-dhcp = true`, or `hostname-template` must be
  present (at least one hostname strategy must be explicitly chosen). This
  replaces the current "hostname is required for metal" rule.
- For the cloud variant, all three fields remain fully optional.

#### Hostname template syntax

The `hostname-template` value is a string containing literal characters and
placeholder expressions enclosed in `{...}`. The supported placeholders are:

- `{hex:N}` — replaced with an N-character lowercase hexadecimal string
  (characters `0-9a-f`), generated from a cryptographically secure random
  source. N must be between 1 and 32 inclusive.
- `{num:N}` — replaced with an N-digit decimal string (characters `0-9`),
  zero-padded to exactly N digits, generated from a cryptographically secure
  random source. N must be between 1 and 10 inclusive.

The literal portions of the template must consist only of characters valid
in hostnames: lowercase ASCII letters, digits, and hyphens. The template
must not start or end with a hyphen, and the total generated hostname must
not exceed 63 characters (the DNS label limit).

Examples:

| Template | Example output |
|----------|---------------|
| `tamanu-{hex:6}` | `tamanu-a3f1b2` |
| `node-{num:4}` | `node-0837` |
| `srv-{hex:4}-{num:3}` | `srv-c0de-042` |
| `{hex:12}` | `3fa82b91c0d4` |

Validation rules:

- The template must contain at least one placeholder.
- The template must not contain unknown placeholders (anything other than
  `hex` or `num`).
- The N parameter must be a positive integer within the allowed range for
  its type.
- The fully expanded hostname (with placeholders replaced at their maximum
  length, which equals N) must not exceed 63 characters.
- The result must be a valid hostname: no leading/trailing hyphens, only
  `[a-z0-9-]` characters.

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
- If the config file has `hostname-template`, the installer generates a
  hostname from the template at startup and pre-fills the text input with
  the result. From the TUI's perspective this behaves exactly as if
  `hostname` had been set to the generated value. The user can still edit
  the pre-filled hostname in interactive/prefilled modes.

For cloud:

- The toggle is not shown. The text input remains optional as today. The
  hint says "Leave empty to skip (default: ubuntu, overridden by
  DHCP/cloud-init)."

The hostname template capability is not exposed in the TUI itself — there
is no screen or toggle for it. It is purely a config-file feature. The TUI
only ever sees the resolved hostname string.

### Installer firstboot logic (`firstboot.rs`)

`apply_hostname` currently writes to `/etc/hostname` and appends to
`/etc/hosts`. The new logic:

- **`hostname` is set** (including hostnames generated from a template):
  write the hostname to `/etc/hostname`, add `127.0.1.1 <hostname>` to
  `/etc/hosts`. (No change from today.)
- **`hostname-from-dhcp` is true**: write an empty file to `/etc/hostname`
  (truncate). Remove any `127.0.1.1` line from `/etc/hosts` if present.
  This restores the metal image's default state, which is useful when the
  installer is used on the cloud image (the cloud image ships with `ubuntu`
  in `/etc/hostname`, and the user explicitly wants DHCP).
- **Neither set (cloud only)**: leave `/etc/hostname` as-is in the written
  image. For cloud, this means `ubuntu` persists. For metal, this means
  the empty file from the image persists. (The installer already requires
  one of the three for metal, so this case only occurs for cloud.)

Note: by the time firstboot runs, a `hostname-template` has already been
resolved to a concrete hostname string and stored in the `hostname` field.
The firstboot logic does not need to know about templates.

### `FirstbootConfig` struct

```rust
pub struct FirstbootConfig {
    pub hostname: Option<String>,
    pub hostname_from_dhcp: bool,       // new field, default false
    pub hostname_template: Option<String>, // new field
    pub tailscale_authkey: Option<String>,
    pub ssh_authorized_keys: Vec<String>,
    pub password: Option<String>,
    pub password_hash: Option<String>,
}
```

`has_hostname_config()` returns true if `hostname.is_some() ||
hostname_from_dhcp || hostname_template.is_some()`.

### Hostname template resolution

Template resolution happens early, during config loading (before the TUI
starts or auto mode proceeds):

1. Parse the template string, extracting literal segments and placeholders.
2. Validate the template (see validation rules above).
3. For each placeholder, generate a random value:
   - `{hex:N}`: generate N random bytes (well, N/2 rounded up), encode as
     lowercase hex, take the first N characters.
   - `{num:N}`: generate a random integer in `[0, 10^N)`, format with
     zero-padding to N digits.
4. Concatenate all segments to produce the final hostname string.
5. Validate the resulting hostname (length <= 63, valid characters, no
   leading/trailing hyphens).
6. Store the result in the `hostname` field of `FirstbootConfig` and clear
   `hostname_template` (the template has been consumed). From this point
   forward, the rest of the installer treats it as a normal static hostname.

The random source must be `getrandom` / `OsRng` (cryptographically secure)
to avoid collisions. Use the `rand` crate's `OsRng` or the `getrandom`
crate directly.

If resolution fails (e.g. the generated hostname is somehow invalid), the
installer must report the error and fall back to interactive mode (same as
other validation errors).

### Dry-run schema

The `firstboot.hostname` field in the install plan JSON changes:

- When a static hostname is set: `"hostname": "server-01"` (string, as
  today).
- When a hostname was generated from a template: the resolved hostname is
  shown, e.g. `"hostname": "tamanu-a3f1b2"`. Additionally, a new field
  `"hostname_from_template"` is set to `true` so tests can distinguish
  template-generated hostnames from manually specified ones.
- When DHCP hostname is chosen: `"hostname": "dhcp"` (the string `"dhcp"`
  as a sentinel).
- When neither is set: `"hostname": null` (as today, cloud-only case).

### Validation

In `InstallConfig::validate()`:

- If more than one of `hostname`, `hostname-from-dhcp`, and
  `hostname-template` is set, emit an error naming the conflicting fields
  (e.g. "firstboot.hostname and firstboot.hostname-template are mutually
  exclusive").
- Existing hostname format validation (length, characters) still applies
  when `hostname` is set.
- `hostname-template` is validated at parse time: the template syntax is
  checked, placeholder types and N values are validated, and the maximum
  expanded length is checked against the 63-character limit.
- `hostname-from-dhcp` on cloud emits a warning: "hostname-from-dhcp has
  no special effect with the cloud variant (DHCP hostname is already the
  default)".

In `mode()`:

- For metal auto, the completeness check becomes: `hostname.is_some() ||
  hostname_from_dhcp || hostname_template.is_some()` (instead of just
  `hostname.is_some()`).

### AppState / TUI state

New fields:

```rust
pub hostname_from_dhcp: bool,       // toggle state, default false
pub hostname_from_template: bool,   // true if hostname was generated from template
```

`hostname_required()` becomes: `self.variant == Variant::Metal &&
!self.hostname_from_dhcp`.

`firstboot_config()` builds the config with:
- `hostname_from_dhcp: self.hostname_from_dhcp`
- `hostname`: from the text input if non-empty (and DHCP toggle is off)

When the config contains `hostname-template`, template resolution runs
during config loading (before TUI init). The resolved hostname is placed
into the text input as a pre-filled value, and `hostname_from_template` is
set to `true`. The TUI then proceeds as normal — the user sees the
generated hostname and can edit it if desired. `hostname_from_template`
is carried through to the dry-run plan output.

### Container test scenarios

Update `test-container-install-all.sh`:

- Add a new scenario: metal with `hostname-from-dhcp = true` and no
  `hostname`. Verify that `/etc/hostname` is empty on the written disk.
- Add a new scenario: metal with `hostname-template = "test-{hex:6}"`.
  Verify that `/etc/hostname` is non-empty and matches the pattern
  `^test-[0-9a-f]{6}$`.
- Update existing metal scenarios that now set hostname to ensure they still
  work.

Update `test-container-install.sh`:

- Add a `SET_HOSTNAME_FROM_DHCP` env var.
- When set, add `hostname-from-dhcp = true` to the generated TOML.
- In the verification section, when `SET_HOSTNAME_FROM_DHCP` is set, check
  that `/etc/hostname` exists and is empty (zero bytes).
- Add a `SET_HOSTNAME_TEMPLATE` env var.
- When set, add `hostname-template = "<value>"` to the generated TOML.
- In the verification section, when `SET_HOSTNAME_TEMPLATE` is set, check
  that `/etc/hostname` is non-empty and matches the expected pattern.

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
| `docs/spec/linux-images.md` | Add `image.hostname.metal-dhcp` and `image.hostname.cloud-default` rules. Add `installer.config.hostname-template` rule. Update `installer.config.schema`, `installer.tui.hostname`, `installer.mode.auto`, `installer.mode.auto-incomplete`, `installer.firstboot.hostname`, `installer.dryrun.schema`. |
| `image/configure.sh` | Make hostname section variant-aware (empty for metal, `ubuntu` for cloud). |
| `installer/tui/src/config.rs` | Add `hostname_from_dhcp` and `hostname_template` to `FirstbootConfig`. Update `validate()` for three-way mutual exclusivity and cloud warning. Update `mode()` completeness check. |
| `installer/tui/src/hostname_template.rs` | New module: template parser (parse placeholders), validator (check syntax/ranges/max-length), and resolver (generate random values, produce final hostname). |
| `installer/tui/src/ui.rs` | Add `hostname_from_dhcp` and `hostname_from_template` to `AppState`. Update `hostname_required()`, `firstboot_config()`, `advance()` logic. Pre-fill hostname from resolved template during init. |
| `installer/tui/src/ui/run.rs` | Handle Tab (focus switch) and Space (toggle) on Hostname screen for metal. |
| `installer/tui/src/ui/render.rs` | Render the DHCP toggle for metal. Dim the text input when toggle is active. Show explanatory note. |
| `installer/tui/src/firstboot.rs` | Handle `hostname_from_dhcp`: truncate `/etc/hostname`, clean `127.0.1.1` from `/etc/hosts`. (Template hostnames are already resolved to plain hostnames by this point.) |
| `installer/tui/src/plan.rs` | Emit `"hostname": "dhcp"` when `hostname_from_dhcp` is true. Emit `"hostname_from_template": true` when hostname was generated from a template. |
| `installer/tui/src/main.rs` | Update auto-incomplete message (hostname, hostname-from-dhcp, or hostname-template required for metal). Add template resolution step before TUI/auto mode proceeds. |
| `installer/tui/tests/dry_run.rs` | Add tests for `hostname-from-dhcp` and `hostname-template` in auto and interactive modes. Update existing tests as needed. |
| `installer/tui/tests/hostname_template.rs` | New test module: unit tests for template parsing, validation (good and bad templates), and resolution (check output matches expected patterns). |
| `tests/test-container-install.sh` | Add `SET_HOSTNAME_FROM_DHCP` and `SET_HOSTNAME_TEMPLATE` env vars, TOML generation, and verification. |
| `tests/test-container-install-all.sh` | Add metal DHCP hostname scenario. Add metal hostname-template scenario. |
| `tests/test-image-structure.sh` | Verify `/etc/hostname` is empty for metal, `ubuntu` for cloud. |
| `README.md` | Update Hostname section and field reference table. Document `hostname-template` syntax and examples. |

## Implementation order

1. Spec updates (all rule changes, including hostname-template).
2. Image build change (variant-aware `/etc/hostname`).
3. Image structure test update.
4. Config schema + validation changes (add both `hostname-from-dhcp` and
   `hostname-template` fields, three-way mutual exclusivity).
5. Hostname template module: parser, validator, resolver.
6. Template resolution integration into config loading / main.
7. Firstboot logic (apply DHCP hostname; template hostnames need no special
   handling since they are already resolved).
8. TUI state + render + key handling (DHCP toggle; template pre-fills the
   hostname text input).
9. Dry-run plan output (DHCP sentinel, template flag).
10. Unit tests for all of the above (including dedicated hostname template
    tests).
11. Dry-run integration tests.
12. Container test scenarios (DHCP and template scenarios).
13. README.
14. Final `cargo clippy`, `cargo fmt`, `tracey query status`.