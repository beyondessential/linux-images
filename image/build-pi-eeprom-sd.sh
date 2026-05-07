#!/bin/bash
# Build the Pi 5 EEPROM-config SD-card artifact.
#
# Produces three loose files (recovery.bin, pieeprom.upd, pieeprom.sig) in
# OUTPUT_DIR, and optionally a flashable raw .img at IMAGE_OUTPUT.
#
# Inputs (env vars):
#   OUTPUT_DIR       Required. Directory for loose files + SHA256SUMS.
#   IMAGE_OUTPUT     Optional. Path for the flashable raw image. If unset,
#                    only loose files are produced.
#   BOOTCONF         Optional. Path to the bootconf.txt to embed.
#                    Default: image/pi-eeprom-config.txt next to this script.
#   RPI_EEPROM_REF   Optional. git ref to clone. Default: a pinned release tag.
#   RPI_EEPROM_REPO  Optional. git URL to clone from.
#                    Default: https://github.com/raspberrypi/rpi-eeprom.git
#   RPI_EEPROM_DIR   Optional. Pre-cloned rpi-eeprom checkout — skips clone.
#   SOURCE_DATE_EPOCH Optional. Used as the timestamp in pieeprom.sig.
#                    Default: mtime of the source pieeprom-*.bin.
#
# Does not require root.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

OUTPUT_DIR="${OUTPUT_DIR:?OUTPUT_DIR is required}"
IMAGE_OUTPUT="${IMAGE_OUTPUT:-}"
BOOTCONF="${BOOTCONF:-$SCRIPT_DIR/pi-eeprom-config.txt}"
RPI_EEPROM_REF="${RPI_EEPROM_REF:-v2025.12.08-2712}"
RPI_EEPROM_REPO="${RPI_EEPROM_REPO:-https://github.com/raspberrypi/rpi-eeprom.git}"
RPI_EEPROM_DIR="${RPI_EEPROM_DIR:-}"

if [ ! -f "$BOOTCONF" ]; then
    echo "ERROR: BOOTCONF not found: $BOOTCONF" >&2
    exit 1
fi

MISSING=()
for cmd in git python3 sha256sum; do
    command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
done
if [ -n "$IMAGE_OUTPUT" ]; then
    for cmd in sfdisk mkfs.vfat mcopy truncate dd; do
        command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
    done
fi
if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "ERROR: missing required commands: ${MISSING[*]}" >&2
    echo "Install: dosfstools mtools util-linux git python3" >&2
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [ -z "$RPI_EEPROM_DIR" ]; then
    echo "Cloning rpi-eeprom @ $RPI_EEPROM_REF ..."
    # Shallow clone of the single ref. Falls back to full fetch if the ref
    # is a commit SHA (--depth+ref only works for branches/tags).
    if ! git clone --depth=1 --branch "$RPI_EEPROM_REF" "$RPI_EEPROM_REPO" "$WORK/rpi-eeprom" 2>/dev/null; then
        git clone "$RPI_EEPROM_REPO" "$WORK/rpi-eeprom"
        git -C "$WORK/rpi-eeprom" checkout "$RPI_EEPROM_REF"
    fi
    RPI_EEPROM_DIR="$WORK/rpi-eeprom"
fi

FW_DIR="$RPI_EEPROM_DIR/firmware-2712/stable"
if [ ! -d "$FW_DIR" ]; then
    echo "ERROR: firmware-2712/stable not found in $RPI_EEPROM_DIR" >&2
    exit 1
fi

SOURCE_PIEEPROM="$(ls -1 "$FW_DIR"/pieeprom-*.bin 2>/dev/null | sort | tail -1)"
if [ -z "$SOURCE_PIEEPROM" ]; then
    echo "ERROR: no pieeprom-*.bin found in $FW_DIR" >&2
    exit 1
fi
SOURCE_RECOVERY="$FW_DIR/recovery.bin"
if [ ! -f "$SOURCE_RECOVERY" ]; then
    echo "ERROR: recovery.bin not found in $FW_DIR" >&2
    exit 1
fi

EEPROM_DATE="$(basename "$SOURCE_PIEEPROM" .bin | sed 's/^pieeprom-//')"
echo "Source EEPROM: pieeprom-$EEPROM_DATE.bin"

STAGE="$WORK/stage"
mkdir -p "$STAGE"

# r[image.pi-eeprom-sd.bootconf+8]
echo "Injecting bootconf into pieeprom.upd ..."
"$RPI_EEPROM_DIR/rpi-eeprom-config" \
    --config "$BOOTCONF" \
    --out "$STAGE/pieeprom.upd" \
    "$SOURCE_PIEEPROM"

# r[image.pi-eeprom-sd.signature+2]
echo "Generating pieeprom.sig ..."
EEPROM_SHA="$(sha256sum "$STAGE/pieeprom.upd" | awk '{print $1}')"
if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
    EEPROM_TS="$SOURCE_DATE_EPOCH"
else
    EEPROM_TS="$(stat -c %Y "$SOURCE_PIEEPROM")"
fi
{
    echo "$EEPROM_SHA"
    echo "ts: $EEPROM_TS"
} > "$STAGE/pieeprom.sig"

cp "$SOURCE_RECOVERY" "$STAGE/recovery.bin"

# r[image.pi-eeprom-sd.artifact]
mkdir -p "$OUTPUT_DIR"
cp "$STAGE/recovery.bin" "$STAGE/pieeprom.upd" "$STAGE/pieeprom.sig" "$OUTPUT_DIR/"
( cd "$OUTPUT_DIR" && sha256sum recovery.bin pieeprom.upd pieeprom.sig > SHA256SUMS )

echo "Loose files written to: $OUTPUT_DIR"
ls -la "$OUTPUT_DIR"

if [ -z "$IMAGE_OUTPUT" ]; then
    exit 0
fi

# r[image.pi-eeprom-sd.flashable+3]
echo "Building flashable image: $IMAGE_OUTPUT"

IMG_TMP="$WORK/sd.img"
# 32 MiB total: 1 MiB pre-partition gap + 31 MiB FAT partition.
# Plenty for recovery.bin (~100 KiB) + pieeprom.upd (2 MiB) + pieeprom.sig.
DISK_SECTORS=$(( 32 * 1024 * 2 ))
PART_START=2048
PART_SECTORS=$(( DISK_SECTORS - PART_START ))
truncate -s 32M "$IMG_TMP"

# MBR partition table, single FAT16 partition starting at 1 MiB.
sfdisk --no-reread --no-tell-kernel "$IMG_TMP" >/dev/null <<SFDISK_EOF
label: dos
unit: sectors
start=$PART_START, size=$PART_SECTORS, type=6, bootable
SFDISK_EOF

# Carve the partition out into a separate file for mkfs/mcopy (avoids
# needing loop devices, keeps the build unprivileged).
PART_TMP="$WORK/sd.part"
dd if="$IMG_TMP" of="$PART_TMP" bs=512 skip=$PART_START count=$PART_SECTORS status=none

# FAT16 with the well-known label. -F 16 is forced — at this size mkfs.vfat
# would otherwise pick FAT12, which the Pi 5 bootloader does not handle.
mkfs.vfat -F 16 -n RECOVERY "$PART_TMP" >/dev/null

# mcopy the three files in (no mount required).
MTOOLS_SKIP_CHECK=1 mcopy -i "$PART_TMP" \
    "$STAGE/recovery.bin" \
    "$STAGE/pieeprom.upd" \
    "$STAGE/pieeprom.sig" \
    ::

# Stitch the formatted partition back into the image.
dd if="$PART_TMP" of="$IMG_TMP" bs=512 seek=$PART_START count=$PART_SECTORS conv=notrunc status=none

mkdir -p "$(dirname "$IMAGE_OUTPUT")"
cp "$IMG_TMP" "$IMAGE_OUTPUT"
echo "Image written: $IMAGE_OUTPUT ($(du -h "$IMAGE_OUTPUT" | cut -f1))"
