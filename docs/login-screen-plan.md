# Login Screen Consolidation Plan

This document describes the plan to consolidate the Tailscale, SSH Keys, and
Password screens into a single "Login" screen with sub-screens.

## Current Flow

```
Welcome -> DiskSelection -> VariantSelection -> [TpmToggle] -> Hostname
  -> Tailscale -> SshKeys -> Password -> Timezone -> NetworkResults -> Confirmation
```

Steps are numbered 1/8 through 8/8 in the header.

## New Flow

```
Welcome -> DiskSelection -> VariantSelection -> [TpmToggle] -> Hostname
  -> Login -> Timezone -> NetworkResults -> Confirmation
```

Steps renumbered 1/6 through 6/6 in the header:

1. Select Target Disk
2. Select Variant (and TPM Configuration, still 2/6)
3. Hostname
4. Login
5. Timezone
6. Confirm

The Login screen replaces three separate screens (Tailscale, SshKeys, Password)
with a single hub screen that has the password entry inline and keybinds to
open sub-screens.

## Login Screen Layout

The Login screen shows:

- Password entry fields (password + confirm), exactly as today's Password screen
- Below the password fields, a list of available actions with keybinds:
  - `t` -- Tailscale auth key (yellow `*` appended if a value is set)
  - `s` -- SSH authorized keys (yellow `*` appended if keys are present)
  - `g` -- Import SSH keys from GitHub (only shown if github.com is reachable)
- Enter/Tab advances from password to confirm, then to the next screen (Timezone)
- Esc goes back to Hostname (or back from confirm to password, as today)

The yellow `*` indicator appears when:
- Tailscale: `tailscale_input.trim()` is non-empty
- SSH keys: `ssh_keys.iter().any(|k| !k.trim().is_empty())`

These indicators update live, so if the user enters a sub-screen, adds a value,
and returns, the `*` appears.

## Sub-Screens

Each sub-screen is a full screen (replaces the Login screen content). Esc or
Enter (where it makes sense) returns to the Login screen.

### Tailscale Sub-Screen (`Screen::LoginTailscale`)

Identical to today's `Screen::Tailscale` content:
- Text input for auth key
- Descriptive text about what the key does
- Enter returns to Login
- Esc returns to Login
- Typing edits the auth key

### SSH Keys Sub-Screen (`Screen::LoginSshKeys`)

A growing list of individual key entry fields, replacing the old single
multi-line text area.

**Data model change**: `ssh_keys_input: String` is replaced by:
```rust
pub ssh_keys: Vec<String>,      // one entry per key field
pub ssh_key_cursor: usize,      // which field is selected
```

Pre-filling from config: each key in `ssh_authorized_keys` becomes one entry
in `ssh_keys`. If the vec is empty, a single empty string is added so there
is always at least one field visible.

GitHub import: fetched keys are appended as new entries in `ssh_keys` (one
per key line returned).

**Visual layout**:
- The selected key field is expanded: a bordered text input showing the full
  key content, with a cursor.
- Non-selected key fields are collapsed to a single line showing a summary:
  `<key-type> <start-of-key>... <comment>` (e.g. `ssh-ed25519 AAAA...BBBB me@host`).
  If the key has no comment, just `<key-type> <start-of-key>...` is shown.
  Empty entries are shown as `(empty)` in gray.

**Key handling**:
- Typing / Backspace: edits the selected key field
- Enter: validate and return to Login screen (filtering out invalid/empty keys)
- Tab: if the current field is non-empty, add a new empty field below and
  select it. If the current field is empty, cycle selection forward to the
  next field (wrapping to the first). This means Tab from the last empty field
  goes back to field 0.
- Shift+Tab: cycle selection backward (wrapping to the last field)
- Esc: return to Login screen (filtering out invalid/empty keys)

**Validation / filtering on exit**: when leaving the SSH keys screen (via Enter
or Esc), entries are filtered: empty strings and strings that don't look like
valid SSH public keys are removed. A minimal validity check is that the line
starts with a recognized key type prefix (`ssh-rsa`, `ssh-ed25519`,
`ssh-dss`, `ecdsa-sha2-nistp256`, `ecdsa-sha2-nistp384`, `ecdsa-sha2-nistp521`,
`sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com`) followed by
a space and at least one more non-whitespace character. Lines that fail this
check are silently dropped. After filtering, if the vec is empty, a single
empty string is re-added so the screen always has at least one field.

**Conversion to `FirstbootConfig`**: `ssh_authorized_keys` is built from
`ssh_keys.iter().filter(|k| !k.trim().is_empty()).map(...)`.

### GitHub Import Sub-Screen (`Screen::LoginGithub`)

Today's GitHub panel from `Screen::SshKeys` as a full screen:
- Text input for GitHub username
- Enter fetches keys; on success each key is appended as a new entry in
  `ssh_keys`, then the screen returns to Login
- Shows fetching/error state inline
- Esc returns to Login

## Screen Enum Changes

Remove:
- `Screen::Tailscale`
- `Screen::SshKeys`
- `Screen::Password`

Add:
- `Screen::Login` -- the hub screen with inline password
- `Screen::LoginTailscale` -- tailscale auth key sub-screen
- `Screen::LoginSshKeys` -- SSH keys sub-screen (growing key field list)
- `Screen::LoginGithub` -- GitHub import sub-screen

## Navigation Changes

### `advance()`

Before:
```
Hostname -> Tailscale -> SshKeys -> Password -> Timezone
```

After:
```
Hostname -> Login -> Timezone
```

The Login sub-screens do not participate in the main `advance()` chain.
They are entered/exited via keybinds on the Login screen.

### `go_back()`

Before:
```
Timezone -> Password -> SshKeys -> Tailscale -> Hostname
```

After:
```
Timezone -> Login -> Hostname
```

From any Login sub-screen, Esc goes back to `Screen::Login` (not to Hostname).

### Header Labels

| Screen             | Label                  |
|--------------------|------------------------|
| DiskSelection      | 1/6 Select Target Disk |
| VariantSelection   | 2/6 Select Variant     |
| TpmToggle          | 2/6 TPM Configuration  |
| Hostname           | 3/6 Hostname           |
| Login              | 4/6 Login              |
| LoginTailscale     | 4/6 Login > Tailscale  |
| LoginSshKeys       | 4/6 Login > SSH Keys   |
| LoginGithub        | 4/6 Login > GitHub     |
| Timezone           | 5/6 Timezone           |
| Confirmation       | 6/6 Confirm            |

### Footer Keybind Hints

| Screen         | Hints                                                             |
|----------------|-------------------------------------------------------------------|
| Login          | `t: tailscale | s: ssh keys | [g: github] | Enter: next | Esc: back` |
| LoginTailscale | `Enter: done | Esc: back`                                         |
| LoginSshKeys   | `Tab: new key / next | Shift+Tab: prev | Enter: done | Esc: back` |
| LoginGithub    | `Enter: fetch keys | Esc: back`                                   |

The `g: github` hint is only shown when github.com is reachable.

## Network Check: Add github.com Endpoint

### Spec Change

Add `https://github.com/` to the endpoint list in `installer.tui.network-check`.
This endpoint expects any HTTP response (not necessarily 200, since GitHub
returns 301). Bump the spec version.

### Implementation

In `net.rs`, add to `default_endpoints()`:
```rust
Endpoint {
    label: "github.com".into(),
    url: "https://github.com/".into(),
    expect_200: false,
}
```

This brings the endpoint count from 5 to 6 (total checks including NTP: 7).

### GitHub Reachability Check

Add a method to `AppState`:
```rust
pub fn github_reachable(&self) -> bool {
    self.net_check_results
        .iter()
        .any(|r| matches!(r, Some(r) if r.label == "github.com" && r.passed))
}
```

The `g` keybind and its hint on the Login screen are only shown/active when
`github_reachable()` returns true.

## Spec Changes

### `installer.tui.network-check` (bump to +4)

Add `https://github.com/` to the endpoint list (any HTTP response is a pass).
Update total count references.

### `installer.tui.tailscale` (bump to +1)

Rewrite: after the hostname screen, the TUI presents a Login screen. The Login
screen has inline password entry and keybinds to open sub-screens for Tailscale
auth key, SSH keys, and GitHub SSH key import. The Tailscale sub-screen is
accessed via `t` from the Login screen.

### `installer.tui.ssh-keys` (bump to +1)

Rewrite: the SSH keys sub-screen is accessed via `s` from the Login screen.
Displays a growing list of individual key entry fields. The selected field is
expanded for editing; non-selected fields are collapsed to a one-line summary
showing the key type, start of the key material, and the comment (if any).
Tab from a non-empty field adds a new field; Tab from an empty field cycles
through existing fields. Shift+Tab cycles backward. Enter or Esc returns to
Login after filtering out empty and invalid entries.

### `installer.tui.ssh-keys.github` (bump to +1)

Rewrite: the GitHub import sub-screen is accessed via `g` from the Login screen,
only when github.com is reachable per the background network checks. Fetches
keys and appends them as individual entries in the SSH keys list, then returns
to Login.

### `installer.tui.password` (bump to +1)

Rewrite: password entry is inline on the Login screen. Same password/confirm
logic as today. The screen is now called "Login" instead of "Password".

### `installer.tui.confirmation` (bump to +3)

Update step numbering from 8/8 to 6/6.

## Files to Modify

### `docs/spec/installer.md`
- Bump and rewrite the five spec items listed above
- Update the endpoint list in network-check

### `installer/tui/src/net.rs`
- Add `github.com` endpoint to `default_endpoints()`
- Update `default_endpoints_has_expected_count` test (5 -> 6)
- Update `total_check_count_includes_ntp` test (6 -> 7)

### `installer/tui/src/ui.rs`
- Replace `Screen::Tailscale`, `Screen::SshKeys`, `Screen::Password` with
  `Screen::Login`, `Screen::LoginTailscale`, `Screen::LoginSshKeys`,
  `Screen::LoginGithub`
- Replace `ssh_keys_input: String` with `ssh_keys: Vec<String>` and
  `ssh_key_cursor: usize`
- Update `advance()` and `go_back()` for new flow
- Add `github_reachable()` method
- Add `filter_ssh_keys()` method that removes empty/invalid entries and
  ensures at least one empty entry remains
- Add `ssh_key_summary(key: &str) -> String` helper for collapsed display
- Remove `ssh_github_focus` field (no longer needed; GitHub is its own screen)
- Update `firstboot_config()` to read from `ssh_keys` vec instead of
  `ssh_keys_input` string
- Update `poll_github_keys()` to append to `ssh_keys` vec
- Update constructor to pre-fill `ssh_keys` from config
- Update all unit tests that reference old screen names, flow, or
  `ssh_keys_input`

### `installer/tui/src/ui/render.rs`
- Remove `render_tailscale()`, `render_ssh_keys()`, `render_password()`
- Add `render_login()` -- password fields + action list with indicators
- Add `render_login_tailscale()` -- reuse tailscale content
- Add `render_login_ssh_keys()` -- growing list of key fields; selected field
  expanded with bordered text input, non-selected collapsed to summary line
- Add `render_login_github()` -- GitHub username input as full screen
- Update `render_header()` step labels (renumber to /6, add sub-screen labels)
- Update `render_footer()` hints for new screens
- Update the `render()` match arms

### `installer/tui/src/ui/run.rs`
- Remove `Screen::Tailscale`, `Screen::SshKeys`, `Screen::Password` key handlers
- Add `Screen::Login` key handler (password input + `t`/`s`/`g` keybinds)
- Add `Screen::LoginTailscale` key handler
- Add `Screen::LoginSshKeys` key handler: Char/Backspace edits selected field,
  Tab adds or cycles forward, Shift+Tab cycles backward, Enter/Esc calls
  `filter_ssh_keys()` and returns to Login
- Add `Screen::LoginGithub` key handler
- Update all scripted tests that navigate through the old flow

### `installer/tui/tests/dry_run/interactive.rs`
- Update all test scripts: remove `Tailscale: skip` / `SshKeys: Tab, Tab` /
  separate Password steps; replace with Login screen navigation
- Tests that set tailscale/ssh values need to use `t`/`s` keybinds from Login

### `installer/tui/tests/dry_run/scripted_navigation.rs`
- Same script updates as interactive.rs

### `installer/tui/tests/dry_run/prefilled.rs`
- Same script updates

### `installer/tui/tests/dry_run/timezone.rs`
- Same script updates (the Tailscale/SshKeys skip steps change)

### `installer/tui/tests/dry_run/hostname.rs`
- Same script updates where full flows are tested

## Implementation Order

1. Update spec (`docs/spec/installer.md`): bump versions, rewrite affected items
2. Stage spec, run `tracey bump`
3. Add `github.com` endpoint to `net.rs`, update net tests
4. Change `Screen` enum in `ui.rs`: add new variants, remove old ones
5. Update `advance()` / `go_back()` in `ui.rs`
6. Add `github_reachable()` to `AppState`
7. Update key handlers in `run.rs`
8. Update render functions in `render.rs`
9. Update unit tests in `ui.rs` (flow tests, screen name references)
10. Update unit tests in `run.rs` (scripted key-event tests)
11. Update integration tests in `tests/dry_run/` (script-based tests)
12. Run `cargo clippy`, `cargo fmt`, `cargo test`, `tracey query status`
13. Delete this plan document
14. Commit