# Plan: Installer Network Configuration

## Status: Draft (updated after merge from main)

## Summary

Add a network configuration screen to the ISO installer, between the Welcome
screen and the Disk Selection screen. This screen lets the user configure
networking for both the live ISO environment (immediate effect) and the
installation target (written to the installed system).

## Motivation

Currently the ISO installer relies entirely on DHCP for the live environment
(provided by `live-boot` with `systemd-networkd`) and installs a DHCP-only
netplan config (`01-all-en-dhcp.yaml`) on the target. The only
network-related customisation is the hostname. This is insufficient for
environments that require static IP addressing, IPv6-only networks, or
intentionally offline installs.

## Current State (post-merge baseline)

The following is already implemented and must be accounted for:

- The **Welcome screen** (`r[installer.tui.welcome+8]`) now includes:
  - `q` to reboot
  - `Ctrl+Alt+d: shell` hint in the footer
  - dm-verity integrity check with progress bar (blocks Enter until passed)
  - `n` keybind to open the standalone NetworkCheck screen
- The **NetworkCheck screen** is reachable from Welcome via `n` and shows
  endpoint connectivity checks + tailscale netcheck in an accordion layout
  (`NetPane::Connectivity` / `NetPane::Tailscale`).
- The **NetworkResults screen** sits between Timezone and Confirmation and
  shows the same check results.
- The netplan file `01-all-en-dhcp.yaml` ships in both the image
  (`image/files/netplan/`) and the ISO rootfs (`iso/rootfs-files/etc/netplan/`).
  It matches `en*` interfaces with `dhcp4: true`, mode 0600.
- `InstallConfig` has no network fields yet. `InstallPlan` /
  `InstallConfigInfo` has no network summary.
- The confirmation screen shows disk, encryption, timezone, hostname,
  tailscale, SSH keys, and password -- but not network config.
- `apply_firstboot` in `firstboot.rs` handles hostname, tailscale, SSH keys,
  password, timezone, and install-log copy -- but not network config.
- The spec already contains `r[installer.config.network-mode]`,
  `r[installer.config.network-static]`, `r[installer.config.iso-network-mode]`,
  `r[installer.tui.network-config+12]`, and `r[installer.finalise.network+3]`
  (added on this branch before the merge).
- The config template (`iso/bes-install.toml.template`) does not yet include
  network fields.

## Screen Flow (before vs after)

### Before (current)

```
Welcome (n: netcheck, q: reboot, verity check) -> DiskSelection -> ...
                \-> NetworkCheck (standalone, Esc back to Welcome)
...
Timezone -> NetworkResults -> Confirmation
```

### After

```
Welcome (q: reboot, verity check) -> NetworkConfig -> DiskSelection -> ...
                                        |
                                        |--- ISO pane (accordion top)
                                        |--- Target pane (accordion bottom)
                                        |--- Alt+c: NetworkCheck
...
Timezone -> NetworkResults -> Confirmation
```

The `n` keybind is removed from the Welcome screen. The standalone
`NetworkCheck` screen is now only reachable via `Alt+c` from the
`NetworkConfig` screen. `Esc` from NetworkCheck returns to NetworkConfig
(not Welcome). The `NetworkResults` screen between Timezone and Confirmation
remains unchanged.

## Network Configuration Screen

### Layout

The screen uses the same accordion pattern as the existing netcheck screen
(`render_net_accordion` in `ui/render.rs`): two panes, one expanded and one
collapsed. The panes are:

1. **Live ISO (current)** -- configures the running live system's network
2. **Installation Target** -- configures what gets written to the installed
   system

### Navigation

- `Tab` moves focus forward through the fields in the active pane. When focus
  is on the last field of the active pane, `Tab` switches the accordion to the
  other pane (expanding it, collapsing the current one) and focuses the first
  field.
- `Shift+Tab` moves focus backward. When focus is on the first field, it
  switches to the other pane and focuses the last field.
- `Enter` from the target pane advances to the next screen (DiskSelection).
- `Esc` goes back to the Welcome screen.
- `Alt+c` opens the NetworkCheck screen (connectivity checks + tailscale
  netcheck). The netcheck is restarted whenever the ISO network configuration
  changes.
- `q` triggers reboot (consistent with all other screens).

### ISO Pane

Contents:

1. An explanatory line: "Configure networking for the current live system."
2. A connectivity status indicator (updated live):
   - "Connected (DHCP on enp0s3, 192.168.1.42/24)" or
   - "No connectivity" or
   - "Configuring..." (during netplan apply)
3. A radio selector for the network mode:
   - `(*) DHCP`
   - `( ) Static IP`
   - `( ) IPv6 SLAAC only`
   - `( ) Offline`
4. If "Static IP" is selected, additional fields appear:
   - **Interface**: dropdown of detected interfaces (from `ip -j link show`,
     filtered to physical interfaces -- those not matching `lo`, `docker*`,
     `veth*`, `br-*`, `tailscale*`)
   - **IP address**: text input accepting CIDR notation (e.g. `192.168.1.10/24`).
     If the user tabs away without a `/xx` suffix, `/24` is appended
     automatically.
   - **Gateway**: text input (e.g. `192.168.1.1`)
   - **DNS** (optional): text input (e.g. `8.8.8.8, 1.1.1.1`)
   - **Search domain** (optional): text input (e.g. `example.com`)

When the mode or any field changes, the live network is reconfigured after a
500ms debounce. Reconfiguration writes a netplan YAML to
`/etc/netplan/90-installer.yaml` and runs `netplan apply`. During apply, the
status shows "Configuring...". After apply, connectivity is re-probed.

When mode is "DHCP", any installer-written netplan file is removed and
`netplan apply` is run to revert to the base DHCP config.

When mode is "Offline", all interfaces are brought down (or the
installer-written netplan is removed and replaced with an empty renderer).

When mode is "IPv6 SLAAC only", the netplan config sets `dhcp4: false`,
`dhcp6: false`, `accept-ra: true` on the selected interface.

### Target Pane

Contents:

1. An explanatory line: "Configure networking for the installed system."
2. A radio selector:
   - `(*) Copy current config` (default)
   - `( ) DHCP`
   - `( ) Static IP`
   - `( ) IPv6 SLAAC only`
   - `( ) Offline`
3. If "Static IP" is selected, the same fields as the ISO pane (interface,
   IP/CIDR, gateway, DNS, search domain). These are independent of the ISO
   pane values.
4. If "Copy current config" is selected, a summary of what will be copied
   is shown (e.g. "Static IP: 192.168.1.10/24 via 192.168.1.1 on enp0s3").

### Default Selection Logic

- ISO pane defaults to "DHCP" (or the value of `iso-network-mode` from the
  config file, if set).
- Target pane defaults to "Copy current config" when no `network-mode` is
  set in the config file. When `network-mode` is set in the config file,
  the target pane pre-selects that concrete mode instead and "Copy current
  config" is not shown (since the config file has expressed a specific
  intent).
- If the ISO pane is changed to "Offline", and the target pane has never been
  touched by the user, the target pane default changes to "DHCP" (since
  "Copy current config" would copy an offline config, which is rarely
  desired). This auto-switch only applies when the target pane is still on
  "Copy current config"; if the user or config file has set a concrete mode,
  it is left alone.

### Offline Target Warning

If "Offline" is selected for the target, when the user presses `Enter` to
advance, a confirmation dialog appears:

```
The target system will have no network configuration.
It will not be reachable after reboot unless configured manually.

Are you sure? (y/n)
```

Pressing `y` advances. Pressing `n` or `Esc` returns to the target pane.

## Connectivity Probing

A background task periodically checks whether the live ISO has network
connectivity. This is lightweight: it checks for a default route and
optionally pings a known endpoint. The result is displayed as the status
line in the ISO pane.

When the ISO network config changes (mode change, field edit after debounce),
the probe restarts and the netcheck (Alt+c) results are invalidated and
restarted.

## Confirmation Screen

The confirmation screen already shows a summary of install-time config (disk,
encryption, timezone, hostname, tailscale, SSH keys, password). It must
additionally show the target network configuration:

```
Network:        DHCP (all Ethernet interfaces)
```
or
```
Network:        Static IP: 192.168.1.10/24 via 192.168.1.1 on enp0s3
                DNS: 8.8.8.8, 1.1.1.1
```
or
```
Network:        IPv6 SLAAC only
```
or
```
Network:        Offline (no network configuration)
```

## Install-Time Application

### Target netplan generation

During `apply_firstboot` (in `firstboot.rs`), the installer writes the target
network configuration to the installed system. This is a new step added after
the existing timezone/hostname/tailscale/ssh/password steps:

- **DHCP** (or "Copy current" when ISO is DHCP): the existing
  `01-all-en-dhcp.yaml` is left as-is (it ships in the base image).
- **Static IP**: a new `/etc/netplan/01-installer-static.yaml` is written
  with the user's settings. The base `01-all-en-dhcp.yaml` is removed.
- **IPv6 SLAAC only**: a new `/etc/netplan/01-installer-ipv6-slaac.yaml` is
  written. The base `01-all-en-dhcp.yaml` is removed.
- **Offline**: the base `01-all-en-dhcp.yaml` is removed. No replacement is
  written.
- **Copy current (static)**: same as static IP, using the ISO pane's values.

### Netplan file permissions

All installer-generated netplan files must have mode 0600 (matching the
existing `01-all-en-dhcp.yaml`, which is verified by
`test-image-structure.sh` and `test-iso-structure.sh`).

## Configuration File (`bes-install.toml`)

New optional fields to add to both `InstallConfig` (in `config.rs`) and the
template (`iso/bes-install.toml.template`):

```toml
# Target network mode: "dhcp" (default), "static", "ipv6-slaac", "offline"
# Note: "copy-current" is a TUI-only option (copies the live ISO config).
# It is not valid in the config file.
network-mode = "static"

# Static IP fields for the target (only used when network-mode = "static")
network-interface = "enp0s3"
network-ip = "192.168.1.10/24"
network-gateway = "192.168.1.1"
network-dns = "8.8.8.8, 1.1.1.1"
network-domain = "example.com"

# ISO/live network mode (optional, rarely needed):
# "dhcp" (default), "static", "ipv6-slaac", "offline"
iso-network-mode = "dhcp"

# Static IP fields for the live ISO (only used when iso-network-mode = "static")
iso-network-interface = "enp0s3"
iso-network-ip = "192.168.1.10/24"
iso-network-gateway = "192.168.1.1"
iso-network-dns = "8.8.8.8, 1.1.1.1"
iso-network-domain = "example.com"
```

When `auto = true`:
- `network-mode` defaults to `"dhcp"` if not set. The network config screen
  is skipped entirely.
- If `iso-network-mode` is set to `"static"`, the ISO network is configured
  before anything else (before even the TUI starts, so that network checks
  work).

When `auto = false` (interactive/prefilled):
- Config file values pre-fill the fields.
- The TUI target pane defaults to "Copy current config" when no
  `network-mode` is set in the config file. When `network-mode` is set,
  the TUI pre-selects that mode instead (no "Copy current" option).
- The user can still change them interactively.

**Note**: The `template_contains_all_config_fields` test in `config.rs`
verifies that every `InstallConfig` field has a corresponding entry in the
template. New fields must be added to both.

## Data Model Changes

### New types

```rust
/// Network configuration mode for a pane (ISO or target).
enum NetworkMode {
    Dhcp,
    StaticIp,
    Ipv6Slaac,
    Offline,
}

/// Network mode for the target pane in the TUI.
/// "CopyCurrent" is TUI-only; it resolves to the effective ISO config
/// before being written to the target. It is never serialised to the
/// config file.
enum TargetNetworkMode {
    CopyCurrent,
    Dhcp,
    StaticIp,
    Ipv6Slaac,
    Offline,
}

/// Static IP configuration fields.
struct StaticNetConfig {
    interface: String,      // selected from dropdown
    ip_cidr: String,        // e.g. "192.168.1.10/24"
    gateway: String,
    dns: String,            // comma-separated, optional
    search_domain: String,  // optional
}

/// Detected network interface for the dropdown.
struct NetInterface {
    name: String,       // e.g. "enp0s3"
    mac: String,        // e.g. "08:00:27:xx:xx:xx"
    state: String,      // "UP", "DOWN", etc.
}
```

### AppState additions (`ui.rs`)

The existing `AppState` struct gains:

```rust
// Network config screen
pub iso_network_mode: NetworkMode,
pub iso_static_config: StaticNetConfig,
pub target_network_mode: TargetNetworkMode,
pub target_static_config: StaticNetConfig,
pub detected_interfaces: Vec<NetInterface>,
pub iso_net_status: NetConnectivityStatus,
pub net_config_pane: NetConfigPane,       // which pane is expanded (Iso / Target)
pub net_config_focus: NetConfigFocus,     // which field has focus
pub net_apply_debounce: Option<Instant>,
pub target_pane_touched: bool,            // has user manually changed target?
```

The existing `net_checks_started` field remains but the checks are now
started when the `NetworkConfig` screen is first shown (not on Welcome
advance).

### InstallConfig additions (`config.rs`)

```rust
pub struct InstallConfig {
    // ... existing fields (auto, disk_encryption, disk, hostname, etc.) ...

    // Config file only stores concrete modes (no CopyCurrent).
    #[serde(default, rename = "network-mode")]
    pub network_mode: Option<NetworkMode>,
    #[serde(default, rename = "network-interface")]
    pub network_interface: Option<String>,
    #[serde(default, rename = "network-ip")]
    pub network_ip: Option<String>,
    #[serde(default, rename = "network-gateway")]
    pub network_gateway: Option<String>,
    #[serde(default, rename = "network-dns")]
    pub network_dns: Option<String>,
    #[serde(default, rename = "network-domain")]
    pub network_domain: Option<String>,

    #[serde(default, rename = "iso-network-mode")]
    pub iso_network_mode: Option<NetworkMode>,
    #[serde(default, rename = "iso-network-interface")]
    pub iso_network_interface: Option<String>,
    #[serde(default, rename = "iso-network-ip")]
    pub iso_network_ip: Option<String>,
    #[serde(default, rename = "iso-network-gateway")]
    pub iso_network_gateway: Option<String>,
    #[serde(default, rename = "iso-network-dns")]
    pub iso_network_dns: Option<String>,
    #[serde(default, rename = "iso-network-domain")]
    pub iso_network_domain: Option<String>,
}
```

### InstallPlan / confirmation additions (`plan.rs`)

The `InstallConfigInfo` struct gains a `network` field that summarises the
target network config for display in the plan JSON and on the confirmation
screen:

```rust
pub struct InstallConfigInfo {
    // ... existing fields ...
    pub network: String,   // e.g. "dhcp", "static:192.168.1.10/24 via 192.168.1.1 on enp0s3"
}
```

The `InstallPlanBuilder` gains a `.network_summary()` setter.

## Screen Enum Changes

```rust
pub enum Screen {
    Welcome,
    NetworkConfig,     // NEW: inserted between Welcome and DiskSelection
    NetworkCheck,      // EXISTING: now reachable from NetworkConfig via Alt+c
    DiskSelection,
    DiskEncryption,
    Hostname,
    HostnameInput,
    Login,
    LoginTailscale,
    LoginSshKeys,
    LoginGithub,
    Timezone,
    NetworkResults,    // EXISTING: unchanged between Timezone and Confirmation
    Confirmation,
    Installing,
    Done,
    Error(String),
}
```

### Navigation changes

`advance()`:
- `Welcome` -> `NetworkConfig` (was `DiskSelection`)
- `NetworkConfig` -> `DiskSelection` (new)
- Everything else unchanged

`go_back()`:
- `NetworkConfig` -> `Welcome` (new)
- `NetworkCheck` -> `NetworkConfig` (was `Welcome`)
- `DiskSelection` -> `NetworkConfig` (was `Welcome`)
- Everything else unchanged

`open_network_check()`:
- Remains, but is now only called from the `NetworkConfig` screen's `Alt+c`
  handler (not from Welcome's `n` handler).

## Affected Files

| File | Changes |
|---|---|
| `docs/spec/installer.md` | Already updated on this branch. Bump `installer.tui.welcome` to +8 (done). |
| `config.rs` | Add network fields to `InstallConfig`, `NetworkMode` enum, validation for static fields, `Deserialize`/`Serialize` impls. |
| `net.rs` | Add `detect_interfaces()`, `apply_netplan()`, connectivity probing, `NetInterface` struct. Existing check/netcheck code unchanged. |
| `ui.rs` | Add `NetworkConfig` to `Screen` enum, new state fields, update `advance()`/`go_back()`, add focus management. |
| `ui/render.rs` | Add `render_network_config()` with dual-pane accordion. Update `render_confirmation()` to show network summary. Update `render()` dispatch. |
| `ui/run.rs` | Add key handling for `NetworkConfig` screen. Remove `n` from Welcome. Wire `Alt+c` in NetworkConfig. Update `run_full_install` to pass network config to `apply_firstboot`. |
| `firstboot.rs` | Add `apply_network_config()` to write target netplan YAML. Call from `apply_firstboot`. |
| `plan.rs` | Add `network` field to `InstallConfigInfo`. Add `.network_summary()` to builder. |
| `iso/bes-install.toml.template` | Add commented-out network fields. |
| `tests/test-container-install.sh` | Add verification for target netplan after install. |
| Test files (`tests/dry_run/*.rs`) | Update scripted navigation tests for new screen. Add network config tests. |

## Implementation Order

1. **Config parsing** (`config.rs`): add `NetworkMode` enum, new
   `InstallConfig` fields, deserialization, validation (static requires
   ip + gateway). Update `bes-install.toml.template`. Ensure
   `template_contains_all_config_fields` test passes.

2. **Net interface detection** (`net.rs`): add `NetInterface` struct,
   `detect_interfaces()` (calls `ip -j link show`, filters), and a simple
   connectivity probe function.

3. **Data model** (`ui.rs`): add `NetworkConfig` to `Screen` enum, add new
   state fields, update `advance()`/`go_back()`/`open_network_check()`.
   Fix all existing tests that assert on navigation (e.g.
   `welcome_advances_to_disk_selection` becomes
   `welcome_advances_to_network_config`,
   `disk_selection_goes_back_to_welcome` becomes
   `disk_selection_goes_back_to_network_config`, etc.).

4. **Network application** (`net.rs`): add `apply_netplan()` for live ISO
   reconfiguration with debounce (writes `/etc/netplan/90-installer.yaml`,
   runs `netplan apply`).

5. **Rendering** (`ui/render.rs`): add `render_network_config()` with both
   panes, radio selectors, field inputs, status indicator, offline warning
   dialog. Re-use `render_net_accordion` pattern from existing netcheck
   rendering.

6. **Key handling** (`ui/run.rs`): add key handling for the `NetworkConfig`
   screen (field navigation, radio selection, text input, Alt+c, Tab/Shift+Tab
   accordion switching, Enter to advance, Esc to go back, offline warning
   dialog y/n). Remove `n` keybind from Welcome (Welcome now only has Enter
   to advance and `q` to reboot; the `n` keybind line in `handle_key` is
   deleted). Ensure network checks are started when entering NetworkConfig
   rather than on Welcome advance.

7. **Firstboot** (`firstboot.rs`): add `apply_network_config()` to write
   target netplan during install. Call it from `apply_firstboot()` after
   existing config steps. Update `run_full_install` to thread through the
   resolved target network config.

8. **Plan/confirmation** (`plan.rs`, `ui/render.rs`): add network config to
   `InstallConfigInfo`, plan JSON, and confirmation screen display.

9. **Tests**: unit tests for config parsing, navigation, mode defaults,
   CIDR auto-suffix, offline warning, target pane copy logic, netplan
   generation. Update all existing scripted navigation tests that are
   affected by the new screen insertion. Update ASCII-only render tests
   for the new screen.

10. **Spec cross-references**: add tracey `r[impl ...]` and `r[verify ...]`
    annotations to all new code, referencing the existing spec rules.

## Test Impact

The following existing tests will need updates after adding the new screen:

**`ui.rs` tests**:
- `welcome_advances_to_disk_selection` -- now advances to `NetworkConfig`
- `open_network_check_from_welcome` -- remove (netcheck no longer from welcome)
- `network_check_goes_back_to_welcome` -- goes back to `NetworkConfig`
- `disk_selection_goes_back_to_welcome` -- goes back to `NetworkConfig`

**`ui/run.rs` tests**:
- `scripted_walk_through_encrypted_flow` -- insert Enter for NetworkConfig
- `scripted_none_encryption_flow` -- insert Enter for NetworkConfig
- `scripted_reboot_on_welcome` -- unchanged (q still works)
- All scripted tests that advance past Welcome need an extra Enter/navigation
  step for NetworkConfig

**`ui/render.rs` tests**:
- Add `network_config_screen_ascii_only` test

## Stale Tracey Annotations

After bumping the spec rule `installer.tui.welcome` from `+7` to `+8` in
the merge resolution, all source-code annotations still reference `+7`.
These sites must be updated to `+8` during implementation (step 3 or 6):

- `ui.rs`: `VerityCheckState` doc, `AppState.verity_check` field,
  `start_verity_check`, `advance()` Welcome arm, and five `#[test]`
  annotations (`initial_state`, `verity_running_blocks_advance`,
  `verity_passed_allows_advance`, `verity_not_needed_allows_advance`,
  `verity_running_still_allows_network_check`)
- `ui/render.rs`: `render_welcome` annotation
- `ui/run.rs`: `handle_key` Welcome arm, `run_tui` verity start,
  `welcome_q_triggers_reboot` test

Run `tracey bump` after staging the spec change to identify all stale sites
automatically.

## Open Questions

None currently. All design decisions have been resolved in the discussion
leading to this plan.