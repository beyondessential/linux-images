#!/bin/bash
#
# Shared helper for mounting the verity-protected images squashfs from an ISO.
#
# Source this file and call iso_images_mount / iso_images_cleanup. The caller
# is responsible for invoking iso_images_cleanup in its own EXIT trap (or
# calling it explicitly before exit).
#
# Usage:
#   source tests/iso-images-mount.sh
#   iso_images_mount "$ISO"          # sets ISO_IMAGES_MNT, ISO_IMAGES_LOOP
#   # ... use $ISO_IMAGES_MNT as the images directory ...
#   iso_images_cleanup               # unmount + veritysetup close + losetup -d
#
# Requires: losetup, sgdisk, partprobe, veritysetup, mount, xorriso, grep.
# Must run as root.

# State variables — callers may read ISO_IMAGES_MNT but must not modify these.
ISO_IMAGES_MNT=""
_ISO_IMAGES_LOOP=""
_ISO_IMAGES_VERITY_NAME="test-iso-images-verity"
_ISO_IMAGES_VERITY_OPEN=0
_ISO_IMAGES_MOUNTED=0
_ISO_IMAGES_GRUB_TMP=""

iso_images_cleanup() {
    set +e
    if [ "$_ISO_IMAGES_MOUNTED" -eq 1 ] && [ -n "$ISO_IMAGES_MNT" ]; then
        umount "$ISO_IMAGES_MNT" 2>/dev/null
        _ISO_IMAGES_MOUNTED=0
    fi
    if [ "$_ISO_IMAGES_VERITY_OPEN" -eq 1 ]; then
        veritysetup close "$_ISO_IMAGES_VERITY_NAME" 2>/dev/null
        _ISO_IMAGES_VERITY_OPEN=0
    fi
    if [ -n "$_ISO_IMAGES_LOOP" ]; then
        losetup -d "$_ISO_IMAGES_LOOP" 2>/dev/null
        _ISO_IMAGES_LOOP=""
    fi
    if [ -n "$_ISO_IMAGES_GRUB_TMP" ]; then
        rm -f "$_ISO_IMAGES_GRUB_TMP"
        _ISO_IMAGES_GRUB_TMP=""
    fi
    if [ -n "$ISO_IMAGES_MNT" ]; then
        rmdir "$ISO_IMAGES_MNT" 2>/dev/null
        ISO_IMAGES_MNT=""
    fi
    set -e
}

# iso_images_mount <iso-file>
#
# Sets up a loop device on the ISO, finds the images partition by GPT type,
# reads the verity trailer and roothash, opens dm-verity, and mounts the
# squashfs. On success, ISO_IMAGES_MNT points to the mounted directory
# containing partitions.json and the raw .img files.
iso_images_mount() {
    local iso="${1:?iso_images_mount: iso file path required}"

    if [ "$(id -u)" -ne 0 ]; then
        echo "ERROR: iso_images_mount requires root" >&2
        return 1
    fi

    if [ ! -f "$iso" ]; then
        echo "ERROR: ISO not found: $iso" >&2
        return 1
    fi

    local missing=()
    for cmd in losetup sgdisk partprobe veritysetup mount xorriso; do
        command -v "$cmd" &>/dev/null || missing+=("$cmd")
    done
    if [ "${#missing[@]}" -gt 0 ]; then
        echo "ERROR: missing required commands: ${missing[*]}" >&2
        return 1
    fi

    # Loop-mount the ISO with partition scanning
    _ISO_IMAGES_LOOP="$(losetup -f --show -P -r "$iso")"
    partprobe "$_ISO_IMAGES_LOOP" 2>/dev/null || true
    udevadm settle 2>/dev/null || true
    sleep 1

    # Find the images partition by GPT type code 8300 (Linux filesystem)
    local images_part=""
    while IFS= read -r line; do
        local partnum code candidate
        partnum="$(echo "$line" | awk '{print $1}')"
        code="$(echo "$line" | awk '{print $6}')"
        if [ "$code" = "8300" ]; then
            candidate="${_ISO_IMAGES_LOOP}p${partnum}"
            if [ -b "$candidate" ]; then
                images_part="$candidate"
                break
            fi
        fi
    done < <(sgdisk -p "$_ISO_IMAGES_LOOP" 2>/dev/null | grep '^ *[0-9]')

    if [ -z "$images_part" ]; then
        echo "ERROR: no Linux filesystem (8300) partition found in ISO GPT" >&2
        iso_images_cleanup
        return 1
    fi

    # Read the verity trailer: last 8 bytes are hash_size as LE u64
    local part_size trailer_bytes hash_size=0 shift=0
    part_size="$(blockdev --getsize64 "$images_part")"
    trailer_bytes="$(dd if="$images_part" bs=1 skip=$((part_size - 8)) count=8 2>/dev/null \
        | od -A n -t u1 | tr -s ' ')"
    for b in $trailer_bytes; do
        hash_size=$((hash_size + (b << shift)))
        shift=$((shift + 8))
    done

    if [ "$hash_size" -eq 0 ] || [ "$hash_size" -ge "$part_size" ]; then
        echo "ERROR: invalid verity trailer on images partition (hash_size=$hash_size, part_size=$part_size)" >&2
        iso_images_cleanup
        return 1
    fi

    local hash_offset=$((part_size - 8 - hash_size))

    # Extract the images verity roothash from grub.cfg inside the ISO
    _ISO_IMAGES_GRUB_TMP="$(mktemp)"
    xorriso -osirrox on -indev "$iso" \
        -extract /boot/grub/grub.cfg "$_ISO_IMAGES_GRUB_TMP" \
        2>/dev/null

    if [ ! -s "$_ISO_IMAGES_GRUB_TMP" ]; then
        echo "ERROR: failed to extract /boot/grub/grub.cfg from ISO" >&2
        iso_images_cleanup
        return 1
    fi

    local roothash
    roothash="$(grep -o 'images\.verity\.roothash=[^ ]*' "$_ISO_IMAGES_GRUB_TMP" \
        | head -1 | cut -d= -f2)"
    rm -f "$_ISO_IMAGES_GRUB_TMP"
    _ISO_IMAGES_GRUB_TMP=""

    if [ -z "$roothash" ]; then
        echo "ERROR: images.verity.roothash not found in grub.cfg" >&2
        iso_images_cleanup
        return 1
    fi

    # Open dm-verity on the images partition
    if ! veritysetup open "$images_part" "$_ISO_IMAGES_VERITY_NAME" \
            "$images_part" "$roothash" \
            --hash-offset="$hash_offset" 2>/dev/null; then
        echo "ERROR: veritysetup open failed for images partition" >&2
        iso_images_cleanup
        return 1
    fi
    _ISO_IMAGES_VERITY_OPEN=1

    # Mount the dm-verity squashfs
    ISO_IMAGES_MNT="$(mktemp -d -t iso-images-mnt-XXXXXX)"
    if ! mount -t squashfs -o ro "/dev/mapper/$_ISO_IMAGES_VERITY_NAME" "$ISO_IMAGES_MNT" 2>/dev/null; then
        echo "ERROR: failed to mount images squashfs via verity" >&2
        iso_images_cleanup
        return 1
    fi
    _ISO_IMAGES_MOUNTED=1

    # Sanity check
    if [ ! -f "$ISO_IMAGES_MNT/partitions.json" ]; then
        echo "ERROR: partitions.json not found in verity-mounted images squashfs" >&2
        iso_images_cleanup
        return 1
    fi

    local img_count
    img_count="$(find "$ISO_IMAGES_MNT" -name '*.img' | wc -l)"
    echo "    Mounted images squashfs via verity ($img_count partition images)"
}
