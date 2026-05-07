# Pi EEPROM-config SD artifact

A standalone, single-purpose artifact: when booted on a Raspberry Pi 5, it
flashes the EEPROM with a known image carrying a fixed bootloader
configuration, then reboots. It is independent of the OS images defined in
`disk-images.md` and is shipped alongside them.

It exists because the public bootloader-recovery images flashed via
`rpi-imager` do not expose the bootloader configuration they bake in; we
need our hardware to ship with a known, reproducible bootconf rather than
whichever defaults the upstream recovery image happened to carry.

## Output forms

> r[image.pi-eeprom-sd.artifact+1]
> The build must produce two equivalent forms:
>
> 1. A directory of loose files: `recovery.bin`, `pieeprom.upd`,
>    `pieeprom.sig`, plus a `SHA256SUMS` file covering them. Users copy
>    these onto an existing FAT-formatted SD card.
> 2. A single flashable raw disk image. Users write it to an SD card with
>    any image-flashing tool. See r[image.pi-eeprom-sd.flashable] for its
>    on-disk layout.
>
> Both forms are architecture-independent — the same files boot on any
> Raspberry Pi 5 regardless of the host that produced them.

## Bootloader configuration

> r[image.pi-eeprom-sd.bootconf+8]
> The EEPROM image embedded in `pieeprom.upd` must carry the following
> configuration (corresponding to `vcgencmd bootloader-config` on the
> running Pi after the artifact has flashed):
>
> - `BOOT_UART=1`
> - `BOOT_ORDER=0xf61` — try SD, then USB, then network; restart on
>   failure
> - `POWER_OFF_ON_HALT=1`
> - `PCIE_PROBE=1`
> - `BOOT_WATCHDOG_TIMEOUT=15`
> - `HDMI_DELAY=0`
> - `PSU_MAX_CURRENT=5000`
>
> All other settings are taken from the source EEPROM image's defaults.

## Signature file format

r[image.pi-eeprom-sd.signature+2]
The `pieeprom.sig` file must contain two lines: the first the lowercase
hex SHA-256 digest of `pieeprom.upd`, and the second the literal `ts: `
followed by a Unix epoch timestamp. This is the form the Pi 5 bootloader
reads when deciding whether the EEPROM image on the recovery medium is
newer than the one currently flashed.

## Flashable image layout

r[image.pi-eeprom-sd.flashable+3]
The flashable raw image must use an MBR partition table with a single
FAT16 partition starting at the 1 MiB offset. The partition must be
formatted as FAT16 with the volume label `RECOVERY` and contain
`recovery.bin`, `pieeprom.upd`, and `pieeprom.sig` at the filesystem root.
The image is sized at most 64 MiB.
