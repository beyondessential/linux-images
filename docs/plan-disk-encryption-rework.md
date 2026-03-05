# Plan: Disk Encryption Rework

## Summary

Rework the installer's encryption flow. Remove the TPM auto-enrollment service
from the metal image entirely. Replace the separate "Variant Selection" and
"TPM Toggle" TUI screens with a single "Disk Encryption" screen that offers
clear options based on hardware detection. Move all key rotation, TPM
enrollment, keyfile setup, and passphrase generation into the installer itself.

## Phase 1: Remove TPM Auto-Enrollment from the Metal Image

Remove the service and script that automatically enroll TPM on first boot.
The image will still ship with LUKS encryption (empty passphrase in slot 0,
empty keyfile, master key re-encryption service), but will no longer attempt
to bind to a TPM on its own.

Files to remove or modify:

- Delete `image/files/setup-tpm-unlock` (the enrollment script).
- Delete `image/files/systemd/setup-tpm-unlock.service`.
- Remove the `setup-tpm-unlock` installation and enablement block from
  `image/configure.sh`.
- Remove `image/files/systemd/luks-reencrypt.service` and its enablement
  from `image/configure.sh` (the installer will handle master key rotation).
- Update `docs/spec/disk-images.md`: remove `image.tpm.service`,
  `image.tpm.enrollment`, `image.tpm.disableable`, and `image.luks.reencrypt`.

The image keeps: LUKS formatting, empty keyfile, crypttab, dracut keyfile
config, and the grow-root-filesystem service.

## Phase 2: Unified "Disk Encryption" TUI Screen

Replace the `VariantSelection` and `TpmToggle` screens with a single
`DiskEncryption` screen. The installer detects whether a TPM is present
(check for `/dev/tpm0` or use `--fake-tpm` flag for testing).

### Screen layout

If a TPM is present:

```
Disk Encryption

  (*) Full-disk encryption, bound to hardware
  ( ) Full-disk encryption, not bound to hardware
  ( ) No encryption
```

Default: bound to hardware.

If no TPM is present:

```
Disk Encryption

  (*) Full-disk encryption, not bound to hardware
  ( ) No encryption
```

Default: not bound to hardware.

Below the radio options, show contextual explanation text based on the
current selection:

- **Bound to hardware**: "The disk's encryption key will be sealed to this
  machine's TPM using PCR 1 (hardware identity: motherboard, CPU, and RAM
  model/serials). The system will boot unattended as long as the hardware
  stays the same. If you move the disk to different hardware, you will need
  the recovery passphrase. Changing the CPU or RAM may also require the
  recovery passphrase."
- **Not bound to hardware**: "A keyfile will be stored on the boot partition
  (xboot or EFI). The system will boot unattended on any hardware. If the
  boot partition is lost, you will need the recovery passphrase."
- **No encryption**: "The root partition will not be encrypted. This is the
  equivalent of the cloud variant."

### Data model changes

Replace the `Variant` enum (`Metal` / `Cloud`) and `disable_tpm: bool` with
a single `DiskEncryption` enum:

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

The image selected for writing is derived the same way (metal or cloud).

### Config file changes

Replace the `variant` and `disable-tpm` fields with a single
`disk-encryption` field:

```toml
# "tpm", "keyfile", or "none"
disk-encryption = "tpm"
```

Keep backward compatibility: if `variant = "metal"` / `variant = "cloud"` is
found (with optional `disable-tpm`), map them to the new model:
- `variant = "cloud"` -> `DiskEncryption::None`
- `variant = "metal"` + `disable-tpm = false` -> `DiskEncryption::TpmBound`
  (if TPM present) or `DiskEncryption::Keyfile` (if not)
- `variant = "metal"` + `disable-tpm = true` -> `DiskEncryption::Keyfile`

### Dry-run / install plan changes

Replace `variant` + `disable_tpm` in the JSON plan with:

```json
{
  "disk_encryption": "tpm" | "keyfile" | "none",
  "variant": "metal" | "cloud"
}
```

Keep `variant` as a derived read-only field for clarity.

## Phase 3: Installer-Side Encryption Setup

After writing the image and expanding partitions, the installer performs all
encryption setup itself (instead of relying on first-boot services). This
only applies when `disk_encryption` is `TpmBound` or `Keyfile` (i.e. the
metal image).

### Step 1: Rotate the LUKS master key

```
cryptsetup reencrypt /dev/disk/by-partlabel/root \
    --key-file /path/to/empty-keyfile \
    --resilience checksum
```

This replaces the `luks-reencrypt.service` that used to run on first boot.
Write `/etc/luks/rotated` marker into the installed system so nothing
attempts re-encryption again.

### Step 2: Enroll the unlock mechanism

**If TpmBound:**

```
systemd-cryptenroll /dev/disk/by-partlabel/root \
    --unlock-key-file=/path/to/empty-keyfile \
    --tpm2-device=auto \
    --tpm2-pcrs=1
```

Note: PCR 1, not PCR 7. PCR 1 covers hardware identity (motherboard, CPU,
RAM model and serials). PCR 7 is Secure Boot state, which is more fragile
across firmware updates.

Update crypttab to use `tpm2-device=auto`.

**If Keyfile:**

Generate a random keyfile (e.g. 4096 bytes from `/dev/urandom`).

```
systemd-cryptenroll /dev/disk/by-partlabel/root \
    --unlock-key-file=/path/to/empty-keyfile \
    --key-file=/path/to/new-keyfile
```

Install the keyfile to `/etc/luks/keyfile` (mode 000) on the installed
system. Update crypttab to reference the keyfile. Update dracut config to
include the new keyfile in the initramfs.

### Step 3: Generate and enroll a recovery passphrase

Generate a human-readable recovery passphrase (e.g. using a wordlist or a
formatted random hex string).

```
systemd-cryptenroll /dev/disk/by-partlabel/root \
    --unlock-key-file=/path/to/empty-keyfile-or-new-key \
    --password
```

(Pipe the passphrase via stdin or use `--new-passphrase`.)

Print the passphrase to the screen for the user to write down. In automatic
mode, print it to stderr.

### Step 4: Remove the empty passphrase slot

Wipe the original empty-passphrase key slot now that we have real unlock
mechanisms in place.

### Step 5: Configure the installed system

Chroot into the installed system and:

- Update `/etc/crypttab` with the appropriate unlock method and a timeout
  (so it falls back to passphrase prompt).
- Rebuild the initramfs with dracut so it picks up the new crypttab and
  (if keyfile mode) the new keyfile.

### Installer TUI: passphrase display

After the write + firstboot phase, before the "Done" screen, show a
"Recovery Passphrase" screen that displays the generated passphrase and
instructs the user to save it. The user must press Enter to acknowledge.
This screen is only shown when disk encryption is TpmBound or Keyfile.

## Phase 4: Spec and Test Updates

- Update `docs/spec/disk-images.md` to reflect removed services and the
  simplified image (no TPM enrollment, no reencrypt service).
- Update `docs/spec/installer.md` to document the new `DiskEncryption`
  screen, config field, and installer-side encryption setup steps.
- Update existing TUI tests that reference `VariantSelection`, `TpmToggle`,
  `disable_tpm`, or `Variant`.
- Add new tests for the `DiskEncryption` screen (TPM present vs absent,
  navigation, selection).
- Add integration tests for the encryption setup steps (mock/loop device).
- Run `tracey bump` after spec changes to mark stale references.

## Migration Notes

- Existing `bes-install.toml` files using `variant` and `disable-tpm` will
  continue to work via the backward-compatibility mapping.
- Already-installed systems with the old `setup-tpm-unlock.service` are
  unaffected (the service is idempotent and self-disabling). New installs
  simply will not have it.
- The change from PCR 7 to PCR 1 for TPM binding is intentional. PCR 7
  (Secure Boot state) breaks on firmware updates. PCR 1 (hardware identity)
  is stable as long as the physical hardware does not change, which is the
  desired behavior for bare-metal servers.

## Order of Implementation

1. Phase 1 (remove auto-enroll from image) -- can ship independently.
2. Phase 2 (new TUI screen + data model) -- requires Phase 1.
3. Phase 3 (installer-side encryption setup) -- requires Phase 2.
4. Phase 4 (spec/test cleanup) -- ongoing alongside each phase.