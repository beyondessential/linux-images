#!/bin/bash
set -ex

# Runs outside of the chroot in the live environment

# Find partitions
DISK=/dev/$(lsblk -ndo PKNAME $(findmnt -no SOURCE /target))
BOOT_PART="/dev/$(lsblk -lno NAME,PARTLABEL $DISK | grep 'xboot' | awk '{print $1}' | head -1)"
EFI_PART="/dev/$(lsblk -lno NAME,PARTLABEL $DISK | grep 'efi' | awk '{print $1}')"
SWAP_PART="/dev/$(lsblk -lno NAME,PARTLABEL $DISK | grep 'swap' | awk '{print $1}')"

: Unmount staging
umount /target/boot/efi
umount /target/boot
umount /target/cdrom
umount /target

: Wipe staging
dd if=/dev/zero of=$SWAP_PART bs=1M status=progress || true

: Remake staging into encrypted swap
dd if=/dev/random of=/var/run/swapkey bs=1 count=64
cryptsetup luksFormat --type luks2 $SWAP_PART --key-file=/var/run/swapkey
cryptsetup open $SWAP_PART swap --key-file=/var/run/swapkey
mkswap /dev/mapper/swap
swapon /dev/mapper/swap

: Mount real root
mount /dev/mapper/root /target -o subvol=@,compress=zstd:6
mount /dev/mapper/root /target/home -o subvol=@home,compress=zstd:6
mount /dev/mapper/root /target/var/log -o subvol=@logs,compress=zstd:6
mount $BOOT_PART /target/boot
mount $EFI_PART /target/boot/efi
mount -t tmpfs tmpfs /target/run
mount -t tmpfs tmpfs /target/tmp
