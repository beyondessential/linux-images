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

## Spec

The spec has already been updated. The requirement `installer.tui.hostname+3`
in `docs/spec/installer.md` describes the full behavior. All 22 existing
references to `+2` across 6 files are now stale and need to be updated as
part of the implementation.

## Stale References

These are the files and lines that reference the old `installer.tui.hostname+2`
and must be updated to `+3` once their code matches the new spec:

### `installer/tui/src/ui.rs`
- line 228: `firstboot_config()` — impl annotation (logic unchanged)
- line 542: `hostname_required()` — impl annotation (logic unchanged)
- line 1013: `hostname_prefilled_from_config` test
- line 1102: `firstboot_config_from_inputs` test
- line 1135: `firstboot_config_empty_strings_are_none` test
- line 1203: `hostname_required_for_metal` test
- line 1211: `hostname_not_required_for_cloud` test
- line 1219: `hostname_not_required_for_metal_with_dhcp` test

### `installer/tui/src/ui/render.rs`
- line 488: `render_hostname()` — needs full rewrite for selector UI

### `installer/tui/src/ui/run.rs`
- line 106: `Screen::Hostname` key handler — needs full rewrite for selector
- line 911: `scripted_metal_empty_hostname_blocks_advance` test
- line 931: `scripted_metal_hostname_typed_allows_advance` test
- line 953: `scripted_cloud_empty_hostname_allows_advance` test
- line 976: `scripted_metal_dhcp_toggle_allows_advance_with_empty_hostname` test
- line 997: `scripted_metal_dhcp_toggle_via_space` test
- line 1017: `scripted_metal_dhcp_toggle_on_off_requires_hostname` test
- line 1040: `scripted_metal_dhcp_on_ignores_typing` test
- line 1065: `scripted_cloud_tab_does_not_toggle_dhcp` test

### `installer/tui/tests/dry_run/hostname.rs`
- line 313: `scripted_metal_dhcp_toggle_produces_dhcp_sentinel` test

### `installer/tui/tests/dry_run/interactive.rs`
- line 132: `interactive_firstboot_fields_captured` test
- line 199: `interactive_empty_firstboot_is_null` test

### `installer/tui/tests/dry_run/scripted_navigation.rs`
- line 212: `scripted_hostname_with_backspace_correction` test

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

## Implementation Checklist

Work through these in order. Each step should be a separate commit.

### Step 1: Add `Screen::HostnameInput` to the `Screen` enum

- [ ] Add `HostnameInput` variant to `Screen` in `installer/tui/src/ui.rs`.
- [ ] Update `advance()` to route `Hostname -> HostnameInput -> Login` when
  static is selected, and `Hostname -> Login` when network-assigned is selected.
- [ ] Update `go_back()` so `HostnameInput` goes back to `Hostname`.
- [ ] Ensure `advance()` handles auto-mode correctly: skip both screens when
  config provides a hostname or hostname-from-dhcp.

### Step 2: Rewrite key handling for `Screen::Hostname` (selector)

- [ ] Replace the current key handler in `installer/tui/src/ui/run.rs` that
  handles text input + Tab/Space DHCP toggle with a simple Up/Down selector.
- [ ] Up/Down toggles `hostname_from_dhcp` (false = Static, true = Network).
- [ ] Enter confirms: if `hostname_from_dhcp`, advance to Login; else advance
  to `HostnameInput`.
- [ ] Esc goes back (already handled by `go_back()`).
- [ ] Bump the `r[impl installer.tui.hostname+2]` annotation to `+3`.

### Step 3: Implement key handling for `Screen::HostnameInput`

- [ ] Handle character input, Backspace, Enter, Esc.
- [ ] On Enter for metal: block advance if hostname is empty, show error.
- [ ] On Enter for cloud: allow empty hostname, advance to Login.
- [ ] Esc returns to `Screen::Hostname`.
- [ ] Annotate with `r[impl installer.tui.hostname+3]`.

### Step 4: Rewrite render functions

- [ ] Rewrite `render_hostname()` in `installer/tui/src/ui/render.rs` to show
  the two-option selector instead of the text input + toggle.
- [ ] Add `render_hostname_input()` for the text input sub-screen.
- [ ] Update footer text for both screens.
- [ ] Keep header step label as `3/6 Hostname` for both.
- [ ] Bump the render annotation to `+3`.

### Step 5: Update unit tests in `installer/tui/src/ui.rs`

- [ ] `hostname_prefilled_from_config` — may only need annotation bump if
  `firstboot_config()` logic is unchanged.
- [ ] `firstboot_config_from_inputs` — annotation bump.
- [ ] `firstboot_config_empty_strings_are_none` — annotation bump.
- [ ] `hostname_required_for_metal` — verify it still passes; annotation bump.
- [ ] `hostname_not_required_for_cloud` — annotation bump.
- [ ] `hostname_not_required_for_metal_with_dhcp` — annotation bump.
- [ ] Bump all `r[verify installer.tui.hostname+2]` to `+3`.

### Step 6: Update scripted unit tests in `installer/tui/src/ui/run.rs`

These tests use input scripts and need their scripts adjusted for the new
two-step flow:

- [ ] `scripted_metal_empty_hostname_blocks_advance` — enter selector (Static
  default) -> Enter -> empty input -> Enter should block. Rewrite script.
- [ ] `scripted_metal_hostname_typed_allows_advance` — enter selector -> Enter
  -> type hostname -> Enter. Rewrite script.
- [ ] `scripted_cloud_empty_hostname_allows_advance` — cloud defaults to
  network-assigned, so this test changes: Enter at selector goes straight to
  Login. Or change to test static path with empty input. Decide based on what
  the test is actually verifying.
- [ ] `scripted_metal_dhcp_toggle_allows_advance_with_empty_hostname` — now:
  Down to select DHCP -> Enter -> should advance to Login. Rewrite script.
- [ ] `scripted_metal_dhcp_toggle_via_space` — Space no longer toggles; this
  test should be replaced with a Down/Enter test.
- [ ] `scripted_metal_dhcp_toggle_on_off_requires_hostname` — now: Down (DHCP)
  -> Up (Static) -> Enter -> empty input -> Enter should block. Rewrite.
- [ ] `scripted_metal_dhcp_on_ignores_typing` — no longer applicable (DHCP
  path never shows text input). Replace with test that DHCP selection skips
  input entirely.
- [ ] `scripted_cloud_tab_does_not_toggle_dhcp` — Tab no longer relevant.
  Replace with cloud selector navigation test.
- [ ] Bump all annotations to `+3`.

### Step 7: Add new unit tests

- [ ] Metal selector: Down selects DHCP, Enter advances to Login (skip input).
- [ ] Metal selector: default (Static) + Enter advances to HostnameInput.
- [ ] Cloud selector: default (network-assigned) + Enter advances to Login.
- [ ] Cloud selector: Up selects Static, Enter advances to HostnameInput.
- [ ] HostnameInput (metal): empty hostname blocks advance.
- [ ] HostnameInput (cloud): empty hostname allowed, advances to Login.
- [ ] HostnameInput: Esc returns to Hostname selector.
- [ ] Config `hostname-from-dhcp = true`: selector starts on network-assigned
  for both variants.
- [ ] Config `hostname = "foo"`: selector starts on Static for both variants.

(Some of these may overlap with rewrites in step 6 — deduplicate as needed.)

### Step 8: Update integration tests (`installer/tui/tests/dry_run/`)

- [ ] `hostname.rs` line 313: `scripted_metal_dhcp_toggle_produces_dhcp_sentinel`
  — rewrite script to use Down + Enter at selector instead of Tab toggle.
- [ ] `interactive.rs` line 132: `interactive_firstboot_fields_captured` — cloud
  variant, currently types hostname directly. Needs: Up (select Static) ->
  Enter -> type hostname -> Enter.
- [ ] `interactive.rs` line 199: `interactive_empty_firstboot_is_null` — cloud
  variant, currently presses Enter to skip hostname. Now: Enter at selector
  (network-assigned default) goes to Login. Script likely needs fewer steps.
- [ ] `scripted_navigation.rs` line 212:
  `scripted_hostname_with_backspace_correction` — cloud variant, currently
  types directly. Needs: Up (select Static) -> Enter -> type + backspace ->
  Enter.
- [ ] Bump all annotations to `+3`.

### Step 9: Final checks

- [ ] `cargo fmt`
- [ ] `cargo clippy`
- [ ] `tracey query status` — confirm no stale references remain and coverage
  is restored.
- [ ] `cargo test` — all tests pass.