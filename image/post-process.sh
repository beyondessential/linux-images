#!/bin/bash
# r[image.postprocess.defrag] r[image.postprocess.dedupe]
# r[build.container-postprocess]: Runs inside a container to isolate
# privileged loopback and device-mapper operations.
#
# Usage: post-process <filestem> <variant>
#   Expects the raw image at /work/<filestem>.raw
set -euo pipefail

FILESTEM="$1"
VARIANT="${2:-metal}"
IMAGE="/work/${FILESTEM}.raw"

if [ ! -f "$IMAGE" ]; then
    echo "ERROR: image not found: $IMAGE"
    exit 1
fi

if [ "$VARIANT" = "metal" ] && [ -e /dev/mapper/image-root ]; then
    echo "ERROR: /dev/mapper/image-root already exists"
    echo "Another LUKS device is using this mapping name"
    exit 1
fi

LOOP_DEVICE=$(losetup -f)

cleanup() {
    set +e
    umount /mnt/image-root 2>/dev/null
    if [ "$VARIANT" = "metal" ]; then
        cryptsetup close image-root 2>/dev/null
    fi
    losetup -d "$LOOP_DEVICE" 2>/dev/null
    rmdir /mnt/image-root 2>/dev/null
}

trap cleanup EXIT
set -x

losetup -P "$LOOP_DEVICE" "$IMAGE"
udevadm settle
sleep 2

# Open the root filesystem (with LUKS if metal variant)
if [ "$VARIANT" = "metal" ]; then
    KEYFILE=$(mktemp)
    truncate -s 0 "$KEYFILE"
    cryptsetup open "${LOOP_DEVICE}p3" image-root --key-file "$KEYFILE"
    rm -f "$KEYFILE"
    BTRFS_DEV="/dev/mapper/image-root"
else
    BTRFS_DEV="${LOOP_DEVICE}p3"
fi

mkdir -p /mnt/image-root
mount -o subvol=@ "$BTRFS_DEV" /mnt/image-root

# r[image.postprocess.cleanup]
rm -rvf /mnt/image-root/etc/cloud/cloud.cfg.d/90-installer-network.cfg
rm -rvf /mnt/image-root/etc/update-motd.d/60-unminimize
truncate -s0 /mnt/image-root/etc/machine-id

# Show initial disk usage
echo "=== Before optimization ==="
btrfs filesystem df /mnt/image-root
compsize -x /mnt/image-root || true

# r[image.postprocess.defrag]: Defragment with zstd level 15
echo "=== Defragmenting with zstd:15 ==="
btrfs filesystem defrag -r -czstd --level 15 /mnt/image-root

echo "=== After defrag ==="
btrfs filesystem df /mnt/image-root
compsize -x /mnt/image-root || true

# r[image.postprocess.dedupe]: Block-level deduplication
echo "=== Deduplicating ==="
duperemove \
    --hashfile=/tmp/dupes \
    --dedupe-options=same \
    --lookup-extents=yes \
    -r -d /mnt/image-root || true

echo "=== After dedupe ==="
btrfs filesystem df /mnt/image-root
compsize -x /mnt/image-root || true

# Clean up
umount /mnt/image-root
if [ "$VARIANT" = "metal" ]; then
    cryptsetup close image-root
fi
losetup -d "$LOOP_DEVICE"
LOOP_DEVICE=""
rmdir /mnt/image-root

trap - EXIT

echo "=== Post-processing complete ==="
