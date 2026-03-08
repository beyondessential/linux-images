#!/bin/bash
#
# Assemble a hybrid live installer ISO from a pre-built rootfs staging
# directory (produced by build-iso-rootfs.sh) and a source disk image.
#
# Inputs:
#   - ROOTFS_DIR: staging directory with live/vmlinuz, live/initrd.img,
#     live/filesystem.squashfs (with verity), live/verity-roothash
#   - SOURCE_IMAGE: disk image (.raw or .raw.zst) to extract partition images from
#
# Output: a hybrid ISO9660 + GPT image with:
#   - ISO9660 filesystem (bootable in VMs as optical media)
#   - El Torito EFI boot catalog with embedded FAT32 ESP image
#   - GPT for USB boot after dd
#   - Appended FAT32 BESCONF partition (writable on USB for bes-install.toml)
#   - Appended images squashfs partition with dm-verity
#   - Squashfs live rootfs with dm-verity
#
# Usage: build-iso.sh
#   Environment variables:
#     ARCH            - amd64 or arm64 (default: amd64)
#     OUTPUT          - output file path (default: output/<arch>/bes-installer-<arch>.iso)
#     ROOTFS_DIR      - path to the rootfs staging directory (required)
#     SOURCE_IMAGE    - path to the source disk image (.raw or .raw.zst) (required)
#     BESCONF_SIZE_MB - BESCONF partition size in MiB (default: 4)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Well-known GPT PARTUUIDs for ISO partitions
# r[impl iso.images-partition+2]
IMAGES_PARTUUID="ac9457d6-7d97-56bc-b6a6-d1bb7a00a45b"
# r[impl iso.config-partition+2]
BESCONF_PARTUUID="e2bac42b-03a7-5048-b8f5-3f6d22100e77"

# Well-known GPT partition type UUIDs
GPT_TYPE_MICROSOFT_BASIC_DATA="EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"
GPT_TYPE_LINUX_FILESYSTEM="0FC63DAF-8483-4772-8E79-3D69D8477DE4"

ARCH="${ARCH:-amd64}"
BESCONF_SIZE_MB="${BESCONF_SIZE_MB:-4}"
BUILD_DATE="$(date -u +%Y-%m-%d)"
ROOTFS_DIR="${ROOTFS_DIR:?ROOTFS_DIR must point to the rootfs staging directory}"
SOURCE_IMAGE="${SOURCE_IMAGE:?SOURCE_IMAGE must point to the source disk image (.raw or .raw.zst)}"
OUTPUT="${OUTPUT:-output/${ARCH}/bes-installer-${ARCH}.iso}"

# r[impl iso.per-arch]
case "$ARCH" in
    amd64)
        GRUB_TARGET="x86_64-efi"
        GRUB_EFI_NAME="BOOTX64.EFI"
        ;;
    arm64)
        GRUB_TARGET="arm64-efi"
        GRUB_EFI_NAME="BOOTAA64.EFI"
        ;;
    *)
        echo "ERROR: ARCH must be amd64 or arm64 (got: $ARCH)"
        exit 1
        ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root"
    exit 1
fi

# Validate rootfs staging directory
for f in live/vmlinuz live/initrd.img live/filesystem.squashfs live/verity-roothash; do
    if [ ! -f "$ROOTFS_DIR/$f" ]; then
        echo "ERROR: rootfs staging directory is missing $f"
        echo "Run build-iso-rootfs.sh first."
        exit 1
    fi
done

if [ ! -f "$SOURCE_IMAGE" ]; then
    echo "ERROR: source image not found: $SOURCE_IMAGE"
    exit 1
fi

MISSING=()
for cmd in mksquashfs sfdisk mkfs.vfat losetup grub-mkimage xorriso zstd jq veritysetup; do
    command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
done
if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "ERROR: missing required commands: ${MISSING[*]}"
    exit 1
fi

LIVE_ROOTHASH="$(cat "$ROOTFS_DIR/live/verity-roothash")"

echo "=============================="
echo "BES Live ISO Assembler"
echo "=============================="
echo "Architecture:  $ARCH"
echo "Output:        $OUTPUT"
echo "Rootfs dir:    $ROOTFS_DIR"
echo "Source image:  $SOURCE_IMAGE"
echo "BESCONF size:  ${BESCONF_SIZE_MB} MiB"
echo "Build date:    $BUILD_DATE"
echo "Live roothash: $LIVE_ROOTHASH"
echo "=============================="
echo ""

# ============================================================
# State tracking for cleanup
# ============================================================
WORK_DIR=""
MNT_ESP=""
EXTRACT_LOOP=""

cleanup() {
    local exit_code=$?
    echo ""
    if [ $exit_code -ne 0 ]; then
        echo "!!! ISO assembly failed (exit code $exit_code), cleaning up..."
    else
        echo "Cleaning up..."
    fi

    set +e

    [ -n "$MNT_ESP" ] && mountpoint -q "$MNT_ESP" 2>/dev/null && umount "$MNT_ESP"
    [ -n "$EXTRACT_LOOP" ] && losetup -d "$EXTRACT_LOOP" 2>/dev/null

    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi

    if [ $exit_code -ne 0 ]; then
        rm -f "$OUTPUT"
    fi
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-iso-XXXXXX)"
MNT_ESP="$WORK_DIR/esp-mnt"
STAGING="$WORK_DIR/staging"

mkdir -p "$MNT_ESP" "$STAGING"

# Copy the pre-built rootfs staging content into our ISO staging tree
cp -a "$ROOTFS_DIR/live" "$STAGING/live"

# ============================================================
# Phase 1: Extract partition images from source image
# ============================================================
# r[impl iso.contents+3]
# r[impl iso.images-partition+2]
echo "==> Phase 1: Extracting partition images from source image..."
IMAGES_STAGING="$WORK_DIR/images-staging"
mkdir -p "$IMAGES_STAGING"

SOURCE_RAW="$WORK_DIR/source.raw"
if [[ "$SOURCE_IMAGE" == *.zst ]]; then
    echo "    Decompressing $SOURCE_IMAGE ..."
    zstd -d "$SOURCE_IMAGE" -o "$SOURCE_RAW"
else
    echo "    Copying $SOURCE_IMAGE (already uncompressed)..."
    cp "$SOURCE_IMAGE" "$SOURCE_RAW"
fi

EXTRACT_LOOP="$(losetup -f --show -P "$SOURCE_RAW")"
echo "    Loop device: $EXTRACT_LOOP"
partprobe "$EXTRACT_LOOP"
udevadm settle
sleep 1

PART_COUNT="$(lsblk -ln -o NAME "$EXTRACT_LOOP" | grep -c "^$(basename "$EXTRACT_LOOP")p")"
if [ "$PART_COUNT" -ne 3 ]; then
    echo "ERROR: expected 3 partitions in source image, got $PART_COUNT"
    exit 1
fi

EFI_PART="${EXTRACT_LOOP}p1"
XBOOT_PART="${EXTRACT_LOOP}p2"
ROOT_PART="${EXTRACT_LOOP}p3"

PART_NAMES=("efi" "xboot" "root")
PART_DEVS=("$EFI_PART" "$XBOOT_PART" "$ROOT_PART")

# Read partition geometry via sfdisk JSON output
SFDISK_JSON="$(sfdisk --json "$EXTRACT_LOOP")"
SECTOR_SIZE="$(echo "$SFDISK_JSON" | jq '.partitiontable.sectorsize')"

PART_TYPES=()
PART_SIZES_MIB=()
for i in 0 1 2; do
    PART_TYPE="$(echo "$SFDISK_JSON" | jq -r ".partitiontable.partitions[$i].type")"
    PART_SIZE_SECTORS="$(echo "$SFDISK_JSON" | jq ".partitiontable.partitions[$i].size")"
    SIZE_MIB=$(( (PART_SIZE_SECTORS * SECTOR_SIZE) / 1048576 ))
    PART_TYPES+=("$PART_TYPE")
    PART_SIZES_MIB+=("$SIZE_MIB")
done

# Root partition uses size_mib=0 to mean "use all remaining space"
PART_SIZES_MIB[2]=0

for idx in 0 1 2; do
    NAME="${PART_NAMES[$idx]}"
    DEV="${PART_DEVS[$idx]}"

    echo "    Extracting $NAME partition from $DEV ..."
    dd if="$DEV" of="$IMAGES_STAGING/${NAME}.img" bs=4M status=none

    IMG_SIZE="$(stat --format='%s' "$IMAGES_STAGING/${NAME}.img")"
    echo "    ${NAME}.img: $(( IMG_SIZE / 1048576 )) MiB"
done

losetup -d "$EXTRACT_LOOP"
EXTRACT_LOOP=""
rm -f "$SOURCE_RAW"

# Generate partitions.json
echo "    Generating partitions.json ..."
jq -n \
    --arg arch "$ARCH" \
    --arg efi_type "${PART_TYPES[0]}" \
    --argjson efi_size "${PART_SIZES_MIB[0]}" \
    --arg xboot_type "${PART_TYPES[1]}" \
    --argjson xboot_size "${PART_SIZES_MIB[1]}" \
    --arg root_type "${PART_TYPES[2]}" \
    --argjson root_size "${PART_SIZES_MIB[2]}" \
    '{
        arch: $arch,
        partitions: [
            { label: "efi",   type_uuid: $efi_type,   size_mib: $efi_size,   image: "efi.img" },
            { label: "xboot", type_uuid: $xboot_type,  size_mib: $xboot_size, image: "xboot.img" },
            { label: "root",  type_uuid: $root_type,   size_mib: $root_size,  image: "root.img" }
        ]
    }' > "$IMAGES_STAGING/partitions.json"

echo "    partitions.json:"
cat "$IMAGES_STAGING/partitions.json"
echo ""

# ============================================================
# Phase 2: Build images squashfs with verity
# ============================================================
# r[impl iso.images-partition+2]
# r[impl iso.verity.images+4]
# r[impl iso.verity.layout+3]
# r[impl iso.verity.build-deps]
echo "==> Phase 2: Building images squashfs with verity..."

IMAGES_SQFS="$WORK_DIR/images.squashfs"
mksquashfs "$IMAGES_STAGING" "$IMAGES_SQFS" \
    -comp zstd -no-exports -noappend -quiet
rm -rf "$IMAGES_STAGING"
echo "    images squashfs: $(du -h "$IMAGES_SQFS" | cut -f1)"

IMAGES_HASHTREE="$WORK_DIR/images.squashfs.hashtree"
VERITY_OUTPUT="$(veritysetup format "$IMAGES_SQFS" "$IMAGES_HASHTREE" 2>&1)"
IMAGES_ROOTHASH="$(echo "$VERITY_OUTPUT" | grep "Root hash:" | awk '{print $NF}')"
echo "    images verity root hash: $IMAGES_ROOTHASH"

# r[impl iso.verity.layout+3]
# Append hash tree + sector-aligned trailer (self-describing verity layout).
# The blob must be padded to a 4096-byte boundary so that the partition size
# reported by blockdev --getsize64 matches the blob size exactly. Without this,
# xorriso silently pads to the sector boundary and the trailer is no longer at
# the end of the partition.
IMAGES_DATA_SIZE="$(stat --format='%s' "$IMAGES_SQFS")"
cat "$IMAGES_HASHTREE" >> "$IMAGES_SQFS"
rm -f "$IMAGES_HASHTREE"

CURRENT_SIZE="$(stat --format='%s' "$IMAGES_SQFS")"
# Total needed: round up (current + 8-byte trailer) to next 4096-byte boundary
TOTAL_NEEDED=$(python3 -c "
cur = $CURRENT_SIZE + 8
aligned = ((cur + 4095) // 4096) * 4096
print(aligned)
")
PADDING=$((TOTAL_NEEDED - CURRENT_SIZE - 8))
if [ "$PADDING" -gt 0 ]; then
    dd if=/dev/zero bs=1 count="$PADDING" 2>/dev/null >> "$IMAGES_SQFS"
fi
# hash_size = distance from end of data to start of trailer
TRAILER_HASH_SIZE=$((TOTAL_NEEDED - 8 - IMAGES_DATA_SIZE))
python3 -c "import struct,sys; sys.stdout.buffer.write(struct.pack('<Q', $TRAILER_HASH_SIZE))" >> "$IMAGES_SQFS"
echo "    images data size:  $IMAGES_DATA_SIZE"
echo "    images total size: $TOTAL_NEEDED (sector-aligned)"
echo "    images blob (sqfs+verity): $(du -h "$IMAGES_SQFS" | cut -f1)"

# ============================================================
# Phase 3: Build GRUB EFI bootloader and ESP image
# ============================================================
# r[impl iso.boot.uefi]
echo "==> Phase 3: Building EFI boot image..."

mkdir -p "$STAGING/EFI/BOOT"
mkdir -p "$STAGING/boot/grub"

grub-mkimage \
    -o "$STAGING/EFI/BOOT/$GRUB_EFI_NAME" \
    -O "$GRUB_TARGET" \
    -p /boot/grub \
    part_gpt part_msdos fat iso9660 normal boot linux configfile loopback \
    search search_label search_fs_uuid search_fs_file ls cat echo test true \
    chain efinet

cat > "$STAGING/boot/grub/grub.cfg" << GRUBCFG
set timeout=1
set default=0

insmod all_video

search --file --no-floppy --set=root /live/vmlinuz

menuentry "BES Installer (${ARCH}, built ${BUILD_DATE})" {
    linux /live/vmlinuz boot=live toram console=tty1 live.verity.roothash=${LIVE_ROOTHASH} images.verity.roothash=${IMAGES_ROOTHASH}
    initrd /live/initrd.img
}

menuentry "BES Installer (${ARCH}, built ${BUILD_DATE}) -- quiet" {
    linux /live/vmlinuz boot=live toram quiet console=tty1 live.verity.roothash=${LIVE_ROOTHASH} images.verity.roothash=${IMAGES_ROOTHASH}
    initrd /live/initrd.img
}
GRUBCFG

# Build a FAT32 image for the El Torito EFI boot catalog entry.
ESP_IMG="$STAGING/boot/efi.img"
ESP_SIZE_MB=16

truncate -s "${ESP_SIZE_MB}M" "$ESP_IMG"
mkfs.vfat -F 12 -n ESP "$ESP_IMG" >/dev/null

mount -o loop "$ESP_IMG" "$MNT_ESP"
mkdir -p "$MNT_ESP/EFI/BOOT"
mkdir -p "$MNT_ESP/boot/grub"
cp "$STAGING/EFI/BOOT/$GRUB_EFI_NAME" "$MNT_ESP/EFI/BOOT/$GRUB_EFI_NAME"
cp "$STAGING/boot/grub/grub.cfg" "$MNT_ESP/boot/grub/grub.cfg"
umount "$MNT_ESP"

echo "    EFI image: $(du -h "$ESP_IMG" | cut -f1)"
echo "    GRUB target: $GRUB_TARGET ($GRUB_EFI_NAME)"

# ============================================================
# Phase 4: Build BESCONF FAT32 partition image
# ============================================================
# r[impl iso.config-partition+2]
echo "==> Phase 4: Building BESCONF partition image..."

BESCONF_IMG="$WORK_DIR/besconf.img"
truncate -s "${BESCONF_SIZE_MB}M" "$BESCONF_IMG"
mkfs.vfat -F 12 -n BESCONF "$BESCONF_IMG" >/dev/null

mount -o loop "$BESCONF_IMG" "$MNT_ESP"
cp "$SCRIPT_DIR/bes-install.toml.template" "$MNT_ESP/bes-install.toml"
umount "$MNT_ESP"

echo "    BESCONF image: $(du -h "$BESCONF_IMG" | cut -f1)"

# ============================================================
# Phase 5: Produce hybrid ISO with xorriso
# ============================================================
# r[impl iso.format]
# r[impl iso.hybrid]
# r[impl iso.usb]
echo "==> Phase 5: Producing hybrid ISO9660 image with xorriso..."

mkdir -p "$(dirname "$OUTPUT")"

xorriso -as mkisofs \
    -o "$OUTPUT" \
    -V "BES_INSTALLER" \
    -R -J \
    -iso-level 3 \
    \
    -e boot/efi.img \
    -no-emul-boot \
    \
    --efi-boot-part --efi-boot-image \
    \
    -append_partition 3 "$GPT_TYPE_LINUX_FILESYSTEM" "$IMAGES_SQFS" \
    \
    -append_partition 4 "$GPT_TYPE_MICROSOFT_BASIC_DATA" "$BESCONF_IMG" \
    \
    "$STAGING"

# ============================================================
# Phase 6: Stamp well-known PARTUUIDs via sfdisk
# ============================================================
# r[impl iso.images-partition+2]
# r[impl iso.config-partition+2]
echo "==> Phase 6: Stamping well-known PARTUUIDs..."

# Find partition numbers by GPT partition name. xorriso names its appended
# partitions "Appended3" and "Appended4" (matching the -append_partition
# numbers). We cannot match by type UUID because xorriso's gap partitions
# share the same Microsoft basic data type as BESCONF.
SFDISK_ISO_JSON="$(sfdisk --json "$OUTPUT")"
IMAGES_PARTNUM="$(echo "$SFDISK_ISO_JSON" | jq -r \
    '.partitiontable.partitions[] | select(.name == "Appended3") | .node' \
    | head -1 | grep -o '[0-9]*$')"
BESCONF_PARTNUM="$(echo "$SFDISK_ISO_JSON" | jq -r \
    '.partitiontable.partitions[] | select(.name == "Appended4") | .node' \
    | head -1 | grep -o '[0-9]*$')"

if [ -z "$IMAGES_PARTNUM" ]; then
    echo "ERROR: could not find images partition (Linux filesystem) in ISO GPT"
    exit 1
fi
if [ -z "$BESCONF_PARTNUM" ]; then
    echo "ERROR: could not find BESCONF partition (Microsoft basic data) in ISO GPT"
    exit 1
fi

sfdisk --part-uuid "$OUTPUT" "$IMAGES_PARTNUM" "$IMAGES_PARTUUID"
echo "    images  partition ${IMAGES_PARTNUM}: PARTUUID=${IMAGES_PARTUUID}"

sfdisk --part-uuid "$OUTPUT" "$BESCONF_PARTNUM" "$BESCONF_PARTUUID"
echo "    besconf partition ${BESCONF_PARTNUM}: PARTUUID=${BESCONF_PARTUUID}"

# Clean up working directory
rm -rf "$WORK_DIR"
WORK_DIR=""

trap - EXIT

echo ""
echo "=============================="
echo "Live ISO built successfully"
echo "=============================="
echo "Output: $OUTPUT"
echo "Size:   $(du -h "$OUTPUT" | cut -f1)"
echo "SHA256: $(sha256sum "$OUTPUT" | cut -d' ' -f1)"
echo ""
echo "Boot in a VM:"
echo "  Attach $OUTPUT as a CD/DVD drive (UEFI mode)"
echo ""
echo "Write to USB:"
echo "  sudo dd if=$OUTPUT of=/dev/sdX bs=4M status=progress"
echo ""
echo "To pre-configure on USB, mount the BESCONF partition and place bes-install.toml:"
echo "  blkid -t PARTUUID=${BESCONF_PARTUUID}   # find the BESCONF partition"
echo "  mount /dev/sdXN /mnt && cp bes-install.toml /mnt/ && umount /mnt"
echo "=============================="
