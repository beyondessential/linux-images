#!/bin/bash
set -ex

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

: Setup LUKS volume on real root
KEYFILE=/tmp/empty-passphrase
touch $KEYFILE
cryptsetup luksFormat --type luks2 $ROOT_PART --key-file $KEYFILE --key-slot 0

: Open LUKS device
cryptsetup open $ROOT_PART root --key-file $KEYFILE
LUKS_DEV="/dev/mapper/root"

: Create filesystem
mkfs.btrfs --label ROOT --checksum xxhash --features block-group-tree,squota $LUKS_DEV

mkdir -p /mnt/newroot
mount $LUKS_DEV /mnt/newroot -o compress=zstd:6
btrfs quota enable --simple /mnt/newroot

: Create subvolumes
btrfs subvolume create /mnt/newroot/@
btrfs subvolume create /mnt/newroot/@home
btrfs subvolume create /mnt/newroot/@logs
btrfs subvolume create /mnt/newroot/@postgres
btrfs subvolume create /mnt/newroot/@containers
btrfs subvolume create /mnt/newroot/@.snapshots

: Copy system from staging to real root
rsync -aAX \
  --exclude=/mnt \
  --exclude=/cdrom \
  --exclude=/boot/\* \
  --exclude=/tmp/\* \
  --exclude=/proc/\* \
  --exclude=/sys/\* \
  --exclude=/dev/\* \
  --exclude=/home \
  --exclude=/var/log \
  / /mnt/newroot/@/

if [ -d /home ] && [ "$(ls -A /home 2>/dev/null)" ]; then
  : Copying /home
  rsync -aAX /home/ /mnt/newroot/@home/
fi

if [ -d /var/log ] && [ "$(ls -A /var/log 2>/dev/null)" ]; then
  : Copying /var/log
  rsync -aAX /var/log/ /mnt/newroot/@logs/
fi

: Fix resolv.conf symlink
rm -f /mnt/newroot/@/etc/resolv.conf
ln -snf /run/systemd/resolve/stuf-resolv.conf /mnt/newroot/@/etc/resolv.conf

: Create mountpoints
mkdir -p /mnt/newroot/@/{.snapshots,boot,dev,home,mnt,proc,root,run,sys,tmp,var/{lib/{postgresql,containers},log}}
: Pre-create directories for snapshots
mkdir -p /mnt/newroot/@snapshots/{root,home,logs,postgres,containers}
