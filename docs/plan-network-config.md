# Plan: Installer Network Configuration

## Status: Draft

## Summary

Add a network configuration screen to the ISO installer, between the Welcome
screen and the Disk Selection screen. This screen lets the user configure
networking for both the live ISO environment (immediate effect) and the
installation target (written to the installed system).

## Motivation

Currently the ISO installer relies entirely on DHCP for the live environment
(provided by `live-boot`) and installs a DHCP-only netplan config
(`01-all-en-dhcp.yaml`) on the target. The only network-related customisation
is the hostname. This is insufficient for environments that require static IP
addressing, IPv6-only networks, or intentionally offline installs.

## Screen Flow (before vs after)

### Before

```
Welcome (n: netcheck) -> DiskSelection -> DiskEncryption -> Hostname -> ...
                \-> NetworkCheck (standalone)
```

### After

```
Welcome -> NetworkConfig -> DiskSelection -> DiskEncryption -> Hostname -> ...
              |                  (Alt+c: netcheck from NetworkConfig)
              |--- ISO pane (accordion top)
              |--- Target pane (accordion bottom)
```

The `n` keybind is removed from the Welcome screen. The standalone
`NetworkCheck` screen is still present but is now only reachable via `Alt+c`
from the `NetworkConfig` screen (and from the `NetworkResults` screen, which
remains between Timezone and Confirmation).

## Network Configuration Screen

### Layout

The screen uses the same accordion pattern as the existing netcheck screen:
two panes, one expanded and one collapsed. The panes are:

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

The confirmation screen (step 6/6) already shows a summary of install-time
config. It must additionally show the target network configuration:

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

During `apply_firstboot`, the installer writes the target network
configuration to the installed system:

- **DHCP** (or "Copy current" when ISO is DHCP): the existing
  `01-all-en-dhcp.yaml` is left as-is (it ships in the image).
- **Static IP**: a new `/etc/netplan/01-static.yaml` is written with the
  user's settings. The base `01-all-en-dhcp.yaml` is removed.
- **IPv6 SLAAC only**: a new `/etc/netplan/01-ipv6-slaac.yaml` is written.
  The base `01-all-en-dhcp.yaml` is removed.
- **Offline**: the base `01-all-en-dhcp.yaml` is removed. No replacement is
  written.
- **Copy current (static)**: same as static IP, using the ISO pane's values.

### Netplan file permissions

All installer-generated netplan files must have mode 0600 (matching the
existing `01-all-en-dhcp.yaml`).

## Configuration File (`bes-install.toml`)

New optional fields:

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

/// Full network configuration for one side (ISO or target).
struct NetworkConfig {
    mode: NetworkMode,  // or TargetNetworkMode for target
    static_config: StaticNetConfig,
}

/// Detected network interface for the dropdown.
struct NetInterface {
    name: String,       // e.g. "enp0s3"
    mac: String,        // e.g. "08:00:27:xx:xx:xx"
    state: String,      // "UP", "DOWN", etc.
}
```

### AppState additions

```rust
pub struct AppState {
    // ... existing fields ...

    // Network config screen
    pub iso_network: NetworkConfig,
    pub target_network: TargetNetworkConfig,
    pub detected_interfaces: Vec<NetInterface>,
    pub iso_net_status: NetConnectivityStatus,
    pub net_config_focus: NetConfigFocus,  // which pane, which field
    pub net_apply_debounce: Option<Instant>,
    pub target_pane_touched: bool,  // has user manually changed target?
}
```

### InstallConfig additions

```rust
pub struct InstallConfig {
    // ... existing fields ...
    // Config file only stores concrete modes (no CopyCurrent).
    pub network_mode: Option<NetworkMode>,
    pub network_config: Option<StaticNetConfig>,
    pub iso_network_mode: Option<NetworkMode>,
    pub iso_network_config: Option<StaticNetConfig>,
}
```

### InstallPlan / confirmation additions

The `InstallConfigInfo` struct gains a `network` field that summarises the
target network config for display in the plan JSON and on the confirmation
screen.

## Screen Enum Changes

```rust
pub enum Screen {
    Welcome,
    NetworkConfig,     // NEW: replaces the welcome->disk transition
    NetworkCheck,      // MOVED: now reachable from NetworkConfig via Alt+c
    DiskSelection,
    DiskEncryption,
    Hostname,
    HostnameInput,
    Login,
    LoginTailscale,
    LoginSshKeys,
    LoginGithub,
    Timezone,
    NetworkResults,
    Confirmation,
    Installing,
    Done,
    Error(String),
}
```

## Implementation Order

1. **Spec update** (`docs/spec/installer.md`): add network config requirements.
2. **Config parsing** (`config.rs`): add new fields, parsing, validation.
3. **Net interface detection** (`net.rs`): add `detect_interfaces()` and
   connectivity probing.
4. **Data model** (`ui.rs`): add new state fields, `NetworkConfig` screen to
   enum, navigation logic (advance/go_back), accordion focus management.
5. **Network application** (`net.rs`): add `apply_netplan()` for live ISO
   reconfiguration with debounce.
6. **Rendering** (`ui/render.rs`): add `render_network_config()` with both
   panes, radio selectors, field inputs, status indicator, offline warning
   dialog.
7. **Key handling** (`ui/run.rs`): add key handling for the `NetworkConfig`
   screen (field navigation, radio selection, text input, Alt+c, Tab/Shift+Tab
   accordion switching, Enter to advance, Esc to go back, offline warning
   dialog y/n).
8. **Firstboot** (`firstboot.rs`): add `apply_network_config()` to write
   target netplan during install.
9. **Plan/confirmation** (`plan.rs`, `ui/render.rs`): add network config to
   plan JSON and confirmation screen display.
10. **Tests**: unit tests for config parsing, navigation, mode defaults,
    CIDR auto-suffix, offline warning, target pane copy logic, netplan
    generation.
11. **Spec cross-references**: add tracey `r[...]` annotations to all new
    code.

## Open Questions

None currently. All design decisions have been resolved in the discussion
leading to this plan.