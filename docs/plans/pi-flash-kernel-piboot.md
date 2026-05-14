# Plan: switch the pi image to flash-kernel + piboot-try

## Why

We currently bypass Ubuntu's `flash-kernel` on pi because, when the pi
variant was first prototyped on noble (24.04), noble's `flash-kernel`
package lacked a Pi 5 db entry. We hand-rolled `bes-pi-firmware-update`
plus a `zz-bes-pi-firmware` kernel postinst hook to fill the gap.

That rationale is now stale:

* pi images are 26.04 (resolute) only — CI builds only `suite: [resolute]`
  for `variant=pi` (`.github/workflows/build.yml:272`).
* `flash-kernel` in resolute (`3.110ubuntu2`) ships the Pi 5B db entry.
  It has actually been present since `3.107ubuntu2` (Oct 2023), so even
  noble has had it via updates for a long time.
* resolute additionally ships `flash-kernel-piboot` (and the supporting
  `piboot-try` package, `1.1ubuntu0.1`) which implements the Pi
  bootloader's A/B "tryboot" mechanism: a bad kernel update auto-rolls
  back to the previous known-good kernel/initrd/DTB on next boot.

Adopting it gives us:

1. Delete `bes-pi-firmware-update` and `zz-bes-pi-firmware` — replaced by
   `flash-kernel`'s own `/etc/kernel/postinst.d/zz-flash-kernel` and
   `piboot-try`'s logic.
2. Boot rollback on the firmware partition (the one part of the image
   that is *not* covered by btrfs + snapper today).

## What `piboot-try` does (so the plan is grounded)

On Pi-5-class hardware (`Method: pi-try` in flash-kernel's db), the
firmware partition is laid out as:

* `/boot/firmware/current/` — known-good boot assets, must always exist.
* `/boot/firmware/new/` — staging area for the next kernel/initrd/DTB
  set. Contains a `state` file: `unknown` → `trying` → `good` / `bad`.
* `/boot/firmware/old/` — backup of the previous known-good set
  (removed and recreated on every flash-kernel run).
* `/boot/firmware/config.txt` — has `os_prefix=current/` under `[all]`
  and `os_prefix=new/` under `[tryboot]`. Plus our existing Pi 5
  settings (UART, I2C, SPI, PCIe gen 3, TPM overlay).
* `/boot/firmware/autoboot.txt` — contains `tryboot_a_b=1` so the EEPROM
  honours the tryboot mode.

Flow on every kernel apt upgrade:

1. dracut generates `/boot/initrd.img-<ver>`.
2. `zz-flash-kernel` postinst hook runs.
3. flash-kernel removes `old/`, populates `new/` with the new
   kernel/initrd/DTB/overlays, writes `new/state=unknown`.
4. On reboot, `piboot-try-reboot.service` sees `state=unknown`, sets it
   to `trying`, and reboots into tryboot mode (which redirects
   `os_prefix` to `new/`).
5. If tryboot fails, the EEPROM falls back to `current/`; on next normal
   boot `piboot-try-reboot.service` writes `state=bad` to inhibit
   further retries.
6. If tryboot boots far enough to start `piboot-try-validate.service`,
   that rotates: `current/` → `old/`, `new/` → `current/`, marks `good`.

flash-kernel itself ships migration: `needs_migrate()` detects the
legacy single-directory layout (no `current/state`, no `os_prefix=` in
config.txt) and `migrate()` rewrites config.txt with the prefix keys,
moves existing assets into `current/`, writes `autoboot.txt`, and
removes the now-stale top-level kernel/initrd/DTB. This is what we lean
on for the on-device migration of existing flashed images.

## Image build changes

### `image/packages.sh`

* Add `flash-kernel-piboot` to the `pi` branch. (It depends on
  `piboot-try`, which provides the actual `/usr/sbin/flash-kernel` and
  the `flash-kernel/db/all.db` with `Method: pi-try`.)
* Remove the stale "Note: no flash-kernel — its noble package lacks a
  Pi 5 db entry" comment.

### `image/files/pi/config.txt`

* Drop `kernel=vmlinuz` and `initramfs initrd.img followkernel`.
  flash-kernel's `pi-try` method picks default filenames (`vmlinuz`
  and `initrd.img`) and the `os_prefix=current/` redirection handles
  the directory. With os_prefix in place, the explicit `kernel=` line
  would resolve to `current/vmlinuz` anyway, but it's redundant.
* Keep the rest (UART, I2C, SPI, PCIe gen 3, TPM overlay,
  `disable_splash`).
* migrate-config will inject `[all] os_prefix=current/` and
  `[tryboot] os_prefix=new/` at the top, plus add
  `dtparam=watchdog=on` at the bottom. Both happen automatically on
  first flash-kernel run.

### `image/configure.sh`

* Reorder the pi branch so config.txt and cmdline.txt are written
  *before* flash-kernel-piboot is installed. (flash-kernel's
  `migrate()` reads the existing config.txt; if the file is missing
  the awk-based migrator fails.)
* Remove the install of `/usr/local/sbin/bes-pi-firmware-update` and
  `/etc/kernel/postinst.d/zz-bes-pi-firmware`.
* Remove the explicit `bes-pi-firmware-update "$KVER"` invocation at
  the end of configure.sh (after dracut). Replace with `flash-kernel`
  (no args) so the freshly-generated initramfs lands in `new/`.

### Files to delete

* `image/files/pi/bes-pi-firmware-update`
* `image/files/pi/zz-bes-pi-firmware`

### Initramfs

dracut writes `/boot/initrd.img-<ver>` and `/boot/vmlinuz-<ver>` (these
are the kernel postinst symlinks/files); flash-kernel reads from there
and copies into `/boot/firmware/new/`. The order in configure.sh is:

1. Install linux-raspi (already happens early).
2. Write our config.txt + cmdline.txt.
3. Install flash-kernel-piboot. Its postinst will run flash-kernel once;
   at this point the initramfs may not have been regenerated since we
   switched to dracut, so the kernel/initrd that lands in `new/` could
   be stale. That's fine — it's overwritten in the next step.
4. Run `dracut --force` to generate a fresh initramfs.
5. Run `flash-kernel` (no args) explicitly. This picks up the fresh
   initramfs, rewrites `new/`, leaves `current/` alone if migration
   already happened.

## Spec changes (`docs/spec/disk-images.md`)

Per AGENTS.md, spec items describe **what**, not **how**. Concrete
edits:

* `r[image.boot.pi-firmware]`: drop "selecting `vmlinuz` as the kernel".
  Replace with a requirement that the firmware partition uses an A/B
  layout where new boot assets are staged separately from the running
  known-good set, and a failed boot of new assets must roll back
  automatically to the previous set.
* `r[image.boot.pi-firmware-update]`: rephrase from "must ship a
  script" to "kernel/initramfs/DTB updates must be propagated to the
  firmware partition on every kernel upgrade, and must not overwrite
  the running known-good boot assets".
* Add a new spec item `r[image.boot.pi-tryboot-rollback]` for the
  rollback-on-failed-boot requirement, separable from the propagation
  requirement.
* The `dtparam=watchdog=on` that migrate-config adds is internal to
  the rollback mechanism — not a spec requirement.

## Migration script for already-flashed images

A standalone shell script: `scripts/migrate-pi-to-piboot.sh`.

Pre-flight checks (each fatal, with explanatory error):

* `/etc/bes/image-variant` reads `pi`.
* `/boot/firmware` is mounted.
* `/usr/local/sbin/bes-pi-firmware-update` exists (i.e. this is an
  old-layout image — script is idempotent if not).
* `rpi-eeprom-update`'s reported FW date is on or after 2025-02-11.
  (Pi 5 / 500 / CM5 require this floor for tryboot per the 26.04
  release notes. If older, point at `rpi-eeprom-update -a` and exit.)

Then:

1. `apt-get update && apt-get install -y flash-kernel-piboot`.
2. Remove `/etc/kernel/postinst.d/zz-bes-pi-firmware` and
   `/usr/local/sbin/bes-pi-firmware-update`.
3. Run `flash-kernel` — this triggers `migrate()`, which moves the
   running kernel/initrd/DTB into `current/`, rewrites config.txt with
   the os_prefix keys, writes `autoboot.txt` with `tryboot_a_b=1`, and
   then populates `new/` with the same assets (so the first reboot
   doesn't try anything new).
4. Verify post-state: `/boot/firmware/current/state` is `good`,
   `/boot/firmware/config.txt` contains `os_prefix=current/`,
   `/boot/firmware/autoboot.txt` contains `tryboot_a_b=1`.
5. Print a reboot instruction.

The script is intentionally one-shot and not packaged — there are only
a handful of flashed images and they can be SSH'd into and run by hand.

## Tests

In `tests/`, add a test that runs against the built pi image and
asserts:

* `/boot/firmware/current/vmlinuz` exists (or whatever filename
  flash-kernel chooses — needs verification).
* `/boot/firmware/current/state` contains `good`.
* `/boot/firmware/config.txt` contains `os_prefix=current/` under
  `[all]` and `os_prefix=new/` under `[tryboot]`.
* `/boot/firmware/autoboot.txt` contains `tryboot_a_b=1`.
* `/boot/firmware/bes-pi-firmware-update` does **not** exist.
* `/etc/kernel/postinst.d/zz-bes-pi-firmware` does **not** exist
  inside the rootfs.

Reuse the existing `tests/` patterns (likely a loop-mount + assertion
script).

## Things deliberately out of scope

* Building a flash-kernel db override for additional `dtparam=` entries
  via `/etc/flash-kernel/db`. Our settings live in `config.txt` which
  flash-kernel preserves — no override needed.
* Pre-resolute (noble) support on pi. We're 26.04-only.
* Packaging the migration script for an apt repo. Manual run is fine
  at our scale.
* Hostonly initramfs on pi — already not enabled (pi takes the
  portable-image dracut path regardless of suite per
  `configure.sh:128-134`); independent of this work.
