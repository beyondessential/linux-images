#!/bin/bash
set -ex

VARIANT=$(cat /tmp/image-variant)

# Find partitions
DISK=$(lsblk -ndo PKNAME $(findmnt -n -o SOURCE /))
ROOT_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'root' | awk '{print $1}')"
STAGING_PART=$(findmnt -n -o SOURCE /)
BOOT_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'xboot' | awk '{print $1}' | head -1)"
EFI_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'efi' | awk '{print $1}')"

if [ "$ROOT_PART" = "/dev/" ]; then
  echo "Partitioning failed"
  exit 1
fi

echo "Disk: $DISK"
echo "Root partition: $ROOT_PART"
echo "Staging partition: $STAGING_PART"
echo "Boot partition: $BOOT_PART"
echo "EFI partition: $EFI_PART"
echo "Variant: $VARIANT"

if [ "$VARIANT" = "metal-encrypted" ]; then
  : Setup LUKS volume on real root
  KEYFILE=/tmp/empty-passphrase
  touch $KEYFILE
  cryptsetup luksFormat --type luks2 $ROOT_PART --key-file $KEYFILE --key-slot 0

  : Open LUKS device
  cryptsetup open $ROOT_PART root --key-file $KEYFILE
  BTRFS_DEV="/dev/mapper/root"
else
  BTRFS_DEV="$ROOT_PART"
fi

: Create filesystem
mkfs.btrfs --label ROOT --checksum xxhash --features block-group-tree,squota $BTRFS_DEV

mkdir -p /mnt/newroot
mount $BTRFS_DEV /mnt/newroot -o compress=zstd:6
btrfs quota enable --simple /mnt/newroot

: Create subvolumes
btrfs subvolume create /mnt/newroot/@
btrfs subvolume create /mnt/newroot/@postgres

: Copy system from staging to real root
rsync -aAX \
  --exclude=/mnt \
  --exclude=/cdrom \
  --exclude=/boot/\* \
  --exclude=/tmp/\* \
  --exclude=/proc/\* \
  --exclude=/sys/\* \
  --exclude=/dev/\* \
  / /mnt/newroot/@/

: Fix resolv.conf symlink
rm -f /mnt/newroot/@/etc/resolv.conf
ln -snf /run/systemd/resolve/stub-resolv.conf /mnt/newroot/@/etc/resolv.conf

: Create mountpoints
mkdir -p /mnt/newroot/@/{boot,dev,mnt,proc,root,run,sys,tmp,var/lib/postgresql}
