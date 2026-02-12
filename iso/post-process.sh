#!/bin/bash
set -euo pipefail

FILESTEM="$1"
VARIANT="${2:-metal-encrypted}"
IMAGE="/work/${FILESTEM}.raw"

if [ "$VARIANT" = "metal-encrypted" ] && [ -e /dev/mapper/image-root ]; then
  echo "ERROR: /dev/mapper/image-root already exists"
  echo "Another LUKS device is using this mapping name"
  exit 1
fi

LOOP_DEVICE=$(losetup -f)

cleanup() {
  umount /mnt/image-root 2>/dev/null || true
  if [ "$VARIANT" = "metal-encrypted" ]; then
    cryptsetup close image-root 2>/dev/null || true
  fi
  losetup -d "$LOOP_DEVICE" 2>/dev/null || true
  rmdir /mnt/image-root 2>/dev/null || true
}

trap cleanup EXIT
set -x

losetup -P "$LOOP_DEVICE" "$IMAGE"
udevadm settle
sleep 2

if [ "$VARIANT" = "metal-encrypted" ]; then
  KEYFILE=$(mktemp)
  trap "rm -f $KEYFILE; cleanup" EXIT
  touch "$KEYFILE"
  cryptsetup open "${LOOP_DEVICE}p4" image-root --key-file "$KEYFILE"
  rm -f "$KEYFILE"
  BTRFS_DEV="/dev/mapper/image-root"
else
  BTRFS_DEV="${LOOP_DEVICE}p4"
fi

mkdir -p /mnt/image-root
mount -o subvol=@ "$BTRFS_DEV" /mnt/image-root

rm -rvf /mnt/image-root/etc/cloud/cloud.cfg.d/90-installer-network.cfg
rm -rvf /mnt/image-root/etc/update-motd.d/60-unminimize
truncate -s0 /mnt/image-root/etc/machine-id

btrfs fi df /mnt/image-root
compsize -x /mnt/image-root
btrfs fi defrag -r -czstd --level 15 /mnt/image-root
btrfs fi df /mnt/image-root
compsize -x /mnt/image-root
duperemove --hashfile=/tmp/dupes --dedupe-options=same --lookup-extents=yes -r -d /mnt/image-root || true
btrfs fi df /mnt/image-root
compsize -x /mnt/image-root

umount /mnt/image-root
if [ "$VARIANT" = "metal-encrypted" ]; then
  cryptsetup close image-root
fi
losetup -d "$LOOP_DEVICE"
rmdir /mnt/image-root

trap - EXIT
