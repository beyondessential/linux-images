# Plan: Pi 5 EEPROM-config SD card artifact

A self-contained SD-card artifact that, when booted on a Pi 5, flashes a known
EEPROM image with a fixed bootconf and reboots. Replaces the opaque
rpi-imager "bootloader" recovery images with one we build, sign, and ship.

## Settings to bake into the EEPROM bootconf

```
[all]
BOOT_UART=1
BOOT_ORDER=0xf61
POWER_OFF_ON_HALT=1
PCIE_PROBE=1
BOOT_WATCHDOG_TIMEOUT=15
HDMI_DELAY=0
PSU_MAX_CURRENT=5000
```

Stored in-repo as `image/pi-eeprom-config.txt`. (Note: no
`NET_INSTALL_AT_POWER_ON` — left at its default.)

## Artifact shape

The Pi 5 BootROM treats any FAT-formatted boot media holding `recovery.bin`
as an EEPROM-recovery image. On boot it loads `recovery.bin`, which checks
whether `pieeprom.sig` matches the running EEPROM and, if not, flashes
`pieeprom.upd` and reboots. After flashing, `recovery.bin` renames itself so
re-flashing doesn't loop.

We produce two output forms:

1. **Loose files** — three files plus checksums, for users who want to drop
   them onto an existing FAT-formatted SD card:
   - `recovery.bin`
   - `pieeprom.upd` — the customised EEPROM image
   - `pieeprom.sig` — sha256 of `pieeprom.upd` plus a `ts:` line

2. **Flashable image** — `bes-pi-eeprom-config-<eeprom-date>.img` (with
   `.zst` companion): a small (~32 MiB) raw image with an MBR partition
   table and a single FAT16 partition containing the three files above.
   Writeable to an SD card with `dd`/rpi-imager/balena-etcher and bootable
   on a Pi 5 directly.

Both forms are arch-independent: they boot on any Pi 5 regardless of host
arch. We still build them on `ubuntu-24.04-arm` for tool/firmware locality
but the artifact itself has no arch suffix.

## Source of EEPROM firmware

`github.com/raspberrypi/rpi-eeprom`, pinned to a release tag. Latest
2712-series tag at time of writing: **`v2025.12.08-2712`**. From that repo:

- `firmware-2712/stable/pieeprom-YYYY-MM-DD.bin` — most-recent dated file
  is the source EEPROM. We pick the lexicographically-greatest match.
- `firmware-2712/stable/recovery.bin` — copied verbatim.
- `rpi-eeprom-config` — used to inject our `bootconf.txt` into the source
  EEPROM image.

Pinning to a release tag makes builds reproducible. Bumping the tag is a
one-line change in the build script (or override via env var).

## Components

### `image/pi-eeprom-config.txt`

The bootconf, exactly as listed above, with a comment explaining each line.

### `image/build-pi-eeprom-sd.sh`

Driver script. Inputs (env vars):

- `OUTPUT_DIR` — where to drop loose files and the optional image
- `IMAGE_OUTPUT` — path for flashable .img (optional; if unset, skip image)
- `RPI_EEPROM_REF` — git ref to clone (default: `v2025.12.08-2712`)
- `RPI_EEPROM_REPO` — override for offline/mirror use

Steps:

1. Clone `rpi-eeprom` at the pinned ref into a working dir (or skip if
   `RPI_EEPROM_DIR` is provided).
2. Locate latest `firmware-2712/stable/pieeprom-*.bin` (sorted) and
   `recovery.bin`.
3. Run `rpi-eeprom-config --config <our-bootconf> --out
   <stage>/pieeprom.upd <pieeprom-source>`.
4. Generate `<stage>/pieeprom.sig`: line 1 = `sha256sum pieeprom.upd`,
   line 2 = `ts: <epoch>` (uses `SOURCE_DATE_EPOCH` if set, else mtime of
   the source pieeprom — keeps reproducibility).
5. Copy `recovery.bin` into stage.
6. Copy `pieeprom.upd`/`pieeprom.sig`/`recovery.bin` to `OUTPUT_DIR/`.
7. If `IMAGE_OUTPUT` set: build the raw .img:
   - 32 MiB truncate
   - sgdisk-equivalent (sfdisk MBR) single FAT16 partition starting at 1 MiB
   - mkfs.vfat with a fixed label `RECOVERY`
   - mcopy the three files in (no loop mount needed, keeps it
     unprivileged — see existing iso build for precedent)
8. SHA256SUMS for the loose-file directory.

### `justfile` recipes

- `pi-eeprom` — build loose files into `output/pi-eeprom/`
- `pi-eeprom-img` — build loose files **and** the .img.zst
- Add `check-deps` entry: needs `git`, `python3`, `mtools` (mcopy/mformat),
  `mkfs.vfat`, `sfdisk`, `zstd`.

### CI: `.github/workflows/build.yml`

New job `pi-eeprom`:

- Runs on `ubuntu-24.04-arm` (single matrix entry; the artifact is
  arch-independent but we keep the build on ARM for consistency with
  `images-pi`).
- Installs `dosfstools`, `mtools`, `util-linux`, `zstd`, `python3`.
- Runs `just pi-eeprom-img`.
- Uploads `pi-eeprom-config-*` artifact (loose files + .img.zst).

Wired into `all-green` and `release`. Release step copies the .img.zst,
loose files, and SHA256SUMS into `release/` and adds them to manifest.json
(format=`pi-eeprom`, variant=`pi-eeprom`, no arch).

### Tests

Structural test (`tests/test-pi-eeprom-img.sh`):

- Loop-mount the .img (or just `mdir`/`mtype` it without root via mtools),
  assert `recovery.bin`, `pieeprom.upd`, `pieeprom.sig` all present.
- `pieeprom.sig` first line is 64-char hex matching
  `sha256sum pieeprom.upd`.
- `pieeprom.sig` second line starts with `ts: ` followed by digits.
- `pieeprom.upd` is exactly 2 MiB (Pi 5 EEPROM size).
- Wired into `just test` for the pi-eeprom variant only via a new
  `test-pi-eeprom` recipe; `test-shellcheck` picks the new script up
  automatically (it scans `image/`, `tests/`, `scripts/`, `iso/`).

### Spec

New file `docs/spec/pi-eeprom-sd.md` describing the artifact at
interface-contract level only:

- The artifact is FAT-formatted media (loose files or .img) containing
  `recovery.bin`, `pieeprom.upd`, `pieeprom.sig`.
- `pieeprom.sig` is the sha256 of `pieeprom.upd` on line 1, `ts: <epoch>`
  on line 2 (this is what the bootloader reads — it's an interface
  contract).
- The list of bootconf settings the artifact must contain. These are
  observable on the live Pi via `vcgencmd bootloader-config` — interface
  contract.
- The .img has an MBR partition table with a single FAT partition
  labelled `RECOVERY` (interface contract for users flashing it).

Tracey r-tags:
- `r[image.pi-eeprom-sd.artifact]` — artifact files exist
- `r[image.pi-eeprom-sd.bootconf]` — bootconf settings are baked in
- `r[image.pi-eeprom-sd.signature]` — signature format
- `r[image.pi-eeprom-sd.flashable]` — partition layout for the .img form
- `r[ci.pi-eeprom-sd-build]` — CI job exists

## Out of scope

- Pi 4 / BCM2711 EEPROM. Easy follow-up (point at `firmware-2711/`) if
  ever needed.
- Customising the EEPROM via on-device `bootconf.txt` rather than
  re-flashing — different use case (runtime override, not persisted).
- Signed EEPROM images (RSA). The `pieeprom.sig` we emit is the unsigned
  sha256+ts form; the bootloader accepts this on stock Pi 5s.
