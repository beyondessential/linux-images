# Hostname Screen Rework Plan

## Problem

When the metal variant is selected, the hostname screen currently shows a text
input with a checkbox below it labelled "Use DHCP hostname (no static hostname)".
This is confusing: the user is presented with a required text input and a toggle
that disables it at the same time, with no clear indication of which path to
choose first. The hint text says "A hostname is required for the metal variant"
even though DHCP is a valid alternative, making it seem like DHCP is a
workaround rather than a first-class option.

For cloud variant, the hostname is currently an optional text input with a note
that it will be overridden by DHCP/cloud-init. This works, but it would be
clearer to present the same selection-first flow so the user explicitly chooses
between setting a static hostname and letting the network assign one.

## Goal

Replace the hostname screen for both variants with a two-step flow:

1. **Selection step**: the user chooses between "Static hostname" and
   "Network-assigned" using an Up/Down selector (same interaction pattern as the
   variant selection screen). The network-assigned option label varies by
   variant to reflect the mechanism used.
2. **Input step** (only if "Static hostname" is chosen): a text input screen
   where the user types the hostname.

If the user selects the network-assigned option, the flow proceeds directly from
the selection step to the Login screen with no text input step.

## Spec Changes

### `installer.tui.hostname` (rewrite)

Replace the current rule with:

> After variant/TPM configuration, the TUI presents a hostname selection
> screen. The screen offers two options via an Up/Down selector:
>
> - **Static hostname**
> - **Network-assigned (DHCP)** (metal variant) or **Network-assigned
>   (DHCP / cloud-init)** (cloud variant)
>
> For the metal variant, "Static hostname" is selected by default. For the
> cloud variant, the network-assigned option is selected by default.
>
> Enter confirms the selection. If "Static hostname" is chosen, a second
> sub-screen (`HostnameInput`) presents a text input for the hostname. The
> field may be pre-filled from the configuration file or a resolved hostname
> template. For the metal variant, the hostname is required: the user must
> enter a non-empty value to advance, and an inline error is shown if the
> field is empty on Enter. For the cloud variant, the hostname is optional:
> if left empty, the image's built-in default hostname (`ubuntu`) is kept.
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

## Screen Variants

Currently there is one `Screen::Hostname`. After this change, add a new
`Screen::HostnameInput` for the static hostname text input.
`Screen::Hostname` becomes the selector screen for both variants:

| Variant | `Screen::Hostname` shows | `Screen::HostnameInput` shows |
|---------|--------------------------|-------------------------------|
| Metal   | Selector: Static (default) / Network-assigned (DHCP) | Text input (required, non-empty) |
| Cloud   | Selector: Static / Network-assigned (DHCP / cloud-init) (default) | Text input (optional, empty OK) |

## State Changes

- Remove the `hostname_from_dhcp` toggle interaction (Tab/Space) from
  `Screen::Hostname` key handling. The toggle is replaced by the selector.
- Reuse `hostname_from_dhcp` to track which option is highlighted in the
  selector. It starts as `false` for metal (Static selected) and `true` for
  cloud (network-assigned selected). The config can override this default:
  `hostname-from-dhcp = true` forces DHCP selected; a non-empty `hostname` or
  `hostname-template` forces Static selected.
- On Enter in the selector:
  - If network-assigned is selected: set `hostname_from_dhcp = true`, advance
    directly to `Screen::Login` (skip `HostnameInput`).
  - If static is selected: set `hostname_from_dhcp = false`, advance to
    `Screen::HostnameInput`.
- `Screen::HostnameInput` handles text input, Backspace, Enter (with non-empty
  validation for metal only), and Esc (back to `Screen::Hostname`).

## Navigation

```
Metal flow:
  ... -> TpmToggle -> Hostname (selector) --(Static)--> HostnameInput -> Login -> ...
                                          \--(DHCP)---> Login -> ...

Cloud flow:
  ... -> VariantSelection -> Hostname (selector) --(Static)--> HostnameInput -> Login -> ...
                                                 \--(Network)-> Login -> ...
```

Esc from `HostnameInput` goes back to `Hostname` (selector).
Esc from `Hostname` goes back to the previous screen (TpmToggle for metal,
VariantSelection for cloud).

## Render Changes

### `Screen::Hostname` (selector, metal)

```
  How should this system get its hostname?

  > Static hostname
    Network-assigned (DHCP)
```

Footer: `Up/Down: select | Enter: next | Esc: back`
Header step: `3/6 Hostname`

### `Screen::Hostname` (selector, cloud)

```
  How should this system get its hostname?

    Static hostname
  > Network-assigned (DHCP / cloud-init)
```

Footer: `Up/Down: select | Enter: next | Esc: back`
Header step: `3/6 Hostname`

### `Screen::HostnameInput` (metal, static chosen)

```
  Enter the hostname for this system.

  Hostname: my-server_
```

If empty on Enter: `"Hostname cannot be empty."` in red.

Footer: `Enter: next | Esc: back`
Header step: `3/6 Hostname`

### `Screen::HostnameInput` (cloud, static chosen)

```
  Enter the hostname for this system.
  Leave empty to keep the default (ubuntu).

  Hostname: _
```

Footer: `Enter: next | Esc: back`
Header step: `3/6 Hostname`

## Config / Prefilled / Auto Mode

- `hostname-from-dhcp = true` in config: selector defaults to network-assigned
  option for both variants. In auto mode, the selector is skipped and DHCP is
  used.
- `hostname = "foo"` in config: selector defaults to Static for both variants,
  and `HostnameInput` is pre-filled with `"foo"`. In auto mode, the selector
  and input are both skipped.
- `hostname-template = "srv-{hex:4}"`: resolved at startup, pre-fills
  `HostnameInput`, selector defaults to Static regardless of variant.
- Auto mode advancement: both screens are skipped (the advance logic jumps
  from `Hostname` past `HostnameInput` when appropriate).

## Test Changes

### Unit tests to update

- `hostname_required_for_metal` — hostname is required only on
  `HostnameInput` when the metal variant reaches it (static chosen).
- `hostname_not_required_for_metal_with_dhcp` — DHCP path never reaches
  `HostnameInput`, so this becomes a navigation test.
- `firstboot_config_with_dhcp_hostname` — unchanged logic, just different UI
  path to reach the same state.
- Tests for advance flow: both variants now go
  `Hostname -> HostnameInput -> Login` or `Hostname -> Login` depending on
  selection.

### Unit tests to add

- Metal selector: Down selects DHCP, Enter advances to Login (skipping input).
- Metal selector: default (Static) + Enter advances to HostnameInput.
- Cloud selector: default (network-assigned) + Enter advances to Login.
- Cloud selector: Up selects Static, Enter advances to HostnameInput.
- HostnameInput (metal): empty hostname blocks advance.
- HostnameInput (cloud): empty hostname allowed, advances to Login.
- HostnameInput: Esc returns to Hostname selector.
- Config `hostname-from-dhcp = true`: selector starts on network-assigned for
  both variants.
- Config `hostname = "foo"`: selector starts on Static for both variants.

### Scripted / integration tests to update

- All metal flow scripts that previously used `Tab` or `Space` to toggle DHCP
  need to use `Down + Enter` to select DHCP instead.
- All metal flow scripts that typed a hostname need an extra `Enter` at the
  selector (Static is default for metal, so Enter goes to input, then type
  hostname, then Enter to advance).
- Cloud flow scripts that previously just typed a hostname need an extra step:
  `Up + Enter` to select Static (since cloud defaults to network-assigned),
  then type hostname, then Enter.
- Cloud flow scripts that previously skipped hostname with an empty Enter now
  just press Enter at the selector (network-assigned is the default for cloud).

## Implementation Steps

1. Update spec in `docs/spec/installer.md`. Run `tracey bump`.
2. Add `Screen::HostnameInput` to the `Screen` enum.
3. Update `advance()` and `go_back()` for the new screen.
4. Implement key handling for selector in `Screen::Hostname` (both variants).
5. Implement key handling for `Screen::HostnameInput`.
6. Implement render functions for both screens.
7. Update footer and header step labels.
8. Update all unit tests.
9. Update all scripted / integration tests.
10. Run `cargo clippy`, `cargo fmt`, `tracey query status`.
11. Commit and push.