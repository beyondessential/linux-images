#!/bin/bash
set -ex

# Runs outside of the chroot in the live environment

# Find partitions
DISK=/dev/$(lsblk -ndo PKNAME $(findmnt -no SOURCE /target))
ROOT_PART="/dev/$(lsblk -lno NAME,PARTLABEL $DISK | grep 'root' | awk '{print $1}')"
BOOT_PART="/dev/$(lsblk -lno NAME,PARTLABEL $DISK | grep 'xboot' | awk '{print $1}' | head -1)"
EFI_PART="/dev/$(lsblk -lno NAME,PARTLABEL $DISK | grep 'efi' | awk '{print $1}')"
SWAP_PART="/dev/$(lsblk -lno NAME,PARTLABEL $DISK | grep 'swap' | awk '{print $1}')"

: Unmounting staging
umount /target/boot/efi
umount /target/boot
umount /target/cdrom
umount /target

: Wiping staging
dd if=/dev/zero of=$SWAP_PART bs=1M status=progress || true

: Remaking staging into encrypted swap
dd if=/dev/random of=/var/run/swapkey bs=1 count=64
cryptsetup luksFormat --type luks2 $SWAP_PART --key-file=/var/run/swapkey
cryptsetup open $SWAP_PART swap-crypt --key-file=/var/run/swapkey
mkswap /dev/mapper/swap-crypt
swapon /dev/mapper/swap-crypt

: Mounting real root
mount $ROOT_PART /target -o subvol=@
mount $ROOT_PART /target/home -o subvol=@home
mount $ROOT_PART /target/var/log -o subvol=@logs
mount $BOOT_PART /mnt/newroot/@/boot
mount $EFI_PART /mnt/newroot/@/boot/efi
