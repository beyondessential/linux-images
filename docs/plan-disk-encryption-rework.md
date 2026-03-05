# Plan: Disk Encryption Rework

## Summary

Rework the installer's encryption flow. Remove the TPM auto-enrollment service
from the metal image entirely. Replace the separate "Variant Selection" and
"TPM Toggle" TUI screens with a single "Disk Encryption" screen that offers
clear options based on hardware detection. Move all key rotation, TPM
enrollment, keyfile setup, and passphrase generation into the installer itself.

All spec changes (`docs/spec/disk-images.md` and `docs/spec/installer.md`)
are done upfront. The implementation phases below reconcile the code and tests
to match the updated specs.

## Spec changes (done)

- `docs/spec/disk-images.md`: removed `image.tpm.service`,
  `image.tpm.enrollment`, `image.tpm.disableable`, and
  `image.luks.reencrypt`. Updated `image.variant.types` to clarify that the
  metal image ships with a placeholder empty passphrase and the installer
  handles all encryption setup.
- `docs/spec/installer.md`:
  - Replaced `variant` / `disable-tpm` config fields with
    `disk-encryption` (`"tpm"`, `"keyfile"`, or `"none"`).
  - Replaced `installer.tui.variant-selection` and `installer.tui.tpm-toggle`
    with `installer.tui.disk-encryption+2` (single screen, TPM detection,
    radio options, contextual explanation text).
  - Updated `installer.mode.auto+2` required fields, auto-incomplete
    conditions, interactive defaults, dry-run schema, confirmation summary.
  - Added `installer.dryrun.fake-tpm` flag.
  - Added `installer.encryption.*` section (overview, key-rotation,
    tpm-enroll, keyfile-enroll, recovery-passphrase, wipe-empty-slot,
    configure-system).
  - Removed `installer.firstboot.tpm-disable`.
  - Updated hostname screen and firstboot.mount to reference disk encryption
    mode instead of variant.

## Phase 1: Remove TPM Auto-Enrollment from the Metal Image

Remove the service and script that automatically enroll TPM on first boot.
The image will still ship with LUKS encryption (empty passphrase in slot 0,
empty keyfile), but will no longer attempt to bind to a TPM or rotate the
master key on its own.

Files to remove:

- `image/files/setup-tpm-unlock`
- `image/files/systemd/setup-tpm-unlock.service`
- `image/files/systemd/luks-reencrypt.service`

Files to modify:

- `image/configure.sh`: remove the `setup-tpm-unlock` and
  `luks-reencrypt` installation/enablement blocks.

The image keeps: LUKS formatting, empty keyfile, crypttab, dracut keyfile
config, and the grow-root-filesystem service.

Run `tracey bump` after to mark stale references.

## Phase 2: Unified "Disk Encryption" TUI Screen and Data Model

Replace the `VariantSelection` and `TpmToggle` screens with a single
`DiskEncryption` screen. Replace the `Variant` enum and `disable_tpm: bool`
with a `DiskEncryption` enum throughout the codebase.

### Data model

```rust
pub enum DiskEncryption {
    TpmBound,       // LUKS + TPM PCR 1
    Keyfile,        // LUKS + keyfile on xboot/EFI
    None,           // no encryption (cloud image)
}
```

The variant written to `/etc/bes/image-variant` is derived:
- `TpmBound` or `Keyfile` -> `metal`
- `None` -> `cloud`

The image selected for writing is derived the same way.

### Config file

Replace `variant` and `disable-tpm` with `disk-encryption`. No backward
compatibility.

### TUI screen

The installer detects whether a TPM is present (check for `/dev/tpm0` or
via `--fake-tpm` flag).

If a TPM is present, three radio options (default: bound to hardware):

1. Full-disk encryption, bound to hardware
2. Full-disk encryption, not bound to hardware
3. No encryption

If no TPM is present, two radio options (default: not bound to hardware):

1. Full-disk encryption, not bound to hardware
2. No encryption

Contextual explanation text below the selection, per the spec.

### Dry-run plan

Replace `variant` + `disable_tpm` with `disk_encryption`, `variant`
(derived), and `tpm_present`.

### Tests to update

- All TUI tests referencing `VariantSelection`, `TpmToggle`, `variant`,
  `disable_tpm`, or `Variant`.
- All config parsing tests referencing `variant` or `disable-tpm`.
- All plan/dry-run tests referencing `variant` or `disable_tpm`.
- Add new tests for `DiskEncryption` screen (TPM present vs absent,
  option cycling, advance/go_back).

## Phase 3: Installer-Side Encryption Setup

After writing the image and expanding partitions, the installer performs all
encryption setup itself. This only applies when `disk_encryption` is
`TpmBound` or `Keyfile`.

### Step 1: Rotate the LUKS master key

`cryptsetup reencrypt` with the empty keyfile. Write `/etc/luks/rotated`
marker.

### Step 2: Enroll the unlock mechanism

**TpmBound:** `systemd-cryptenroll --tpm2-pcrs=1`. Update crypttab to
`tpm2-device=auto`.

PCR 1 (hardware identity), not PCR 7 (Secure Boot state).

**Keyfile:** Generate 4096-byte random keyfile, enroll via
`systemd-cryptenroll`, install to `/etc/luks/keyfile` (mode 000). Update
crypttab and dracut config.

### Step 3: Generate and enroll a recovery passphrase

Generate a human-readable passphrase, enroll as a LUKS password slot.
Display to user (TUI screen in interactive mode, stderr in auto mode).

### Step 4: Remove the empty passphrase slot

Wipe the original empty-passphrase key slot.

### Step 5: Configure the installed system

Chroot and rebuild initramfs with dracut.

### TUI: Recovery Passphrase screen

New screen after write/firstboot, before "Done". Only shown when encryption
is enabled. User must press Enter to acknowledge.

## Migration Notes

- Existing `bes-install.toml` files using `variant` and `disable-tpm` must
  be updated to use the new `disk-encryption` field. The old fields are
  removed with no backward compatibility.
- Already-installed systems with the old `setup-tpm-unlock.service` are
  unaffected (the service is idempotent and self-disabling). New installs
  simply will not have it.
- The change from PCR 7 to PCR 1 for TPM binding is intentional. PCR 7
  (Secure Boot state) breaks on firmware updates. PCR 1 (hardware identity)
  is stable as long as the physical hardware does not change, which is the
  desired behavior for bare-metal servers.

## Order of Implementation

1. Phase 1 (remove auto-enroll from image).
2. Phase 2 (new TUI screen + data model + config + tests).
3. Phase 3 (installer-side encryption setup + recovery passphrase screen).