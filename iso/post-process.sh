#!/bin/bash
set -euo pipefail

if [ -e /dev/mapper/image-root ]; then
  echo "ERROR: /dev/mapper/image-root already exists"
  echo "Another LUKS device is using this mapping name"
  exit 1
fi

LOOP_DEVICE=$(losetup -f)
IMAGE="/work/$1.raw"

cleanup() {
  umount /mnt/image-root 2>/dev/null || true
  cryptsetup close image-root 2>/dev/null || true
  losetup -d "$LOOP_DEVICE" 2>/dev/null || true
  rmdir /mnt/image-root 2>/dev/null || true
}

trap cleanup EXIT
set -x

losetup -P "$LOOP_DEVICE" "$IMAGE"
udevadm settle
sleep 2

KEYFILE=$(mktemp)
trap "rm -f $KEYFILE; cleanup" EXIT
touch "$KEYFILE"
cryptsetup open "${LOOP_DEVICE}p4" image-root --key-file "$KEYFILE"
rm -f "$KEYFILE"

mkdir -p /mnt/image-root
mount -o subvol=@ /dev/mapper/image-root /mnt/image-root

rm -rvf /mnt/image-root/etc/cloud/cloud.cfg.d/90-installer-network.cfg
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
cryptsetup close image-root
losetup -d "$LOOP_DEVICE"
rmdir /mnt/image-root

trap - EXIT
