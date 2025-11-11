#!/bin/bash
set -ex

# Migration script to move installed system from staging partition to BTRFS root
# This runs in the autoinstall late-commands phase

# Find partitions
DISK=$(lsblk -ndo PKNAME $(findmnt -n -o SOURCE /))
ROOT_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'root' | awk '{print $1}')"
STAGING_PART=$(findmnt -n -o SOURCE /)
BOOT_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'boot' | awk '{print $1}' | head -1)"
EFI_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'efi' | awk '{print $1}')"

echo "Disk: $DISK"
echo "Root partition: $ROOT_PART"
echo "Staging partition: $STAGING_PART"
echo "Boot partition: $BOOT_PART"
echo "EFI partition: $EFI_PART"

# Format root partition as BTRFS with features enabled
echo "Creating BTRFS filesystem with features..."
mkfs.btrfs -f -L ROOT --features block-group-tree,squota $ROOT_PART

# Mount and create subvolumes
mkdir -p /mnt/newroot
mount $ROOT_PART /mnt/newroot

echo "Creating BTRFS subvolumes..."
btrfs subvolume create /mnt/newroot/@
btrfs subvolume create /mnt/newroot/@home
btrfs subvolume create /mnt/newroot/@logs
btrfs subvolume create /mnt/newroot/@postgres
btrfs subvolume create /mnt/newroot/@containers
btrfs subvolume create /mnt/newroot/snapshots

# Enable quotas
echo "Enabling quotas..."
btrfs quota enable /mnt/newroot

# Copy system from staging to BTRFS subvolumes
echo "Copying system to BTRFS subvolumes..."
rsync -aAX \
  --exclude=/mnt \
  --exclude=/tmp/* \
  --exclude=/proc/* \
  --exclude=/sys/* \
  --exclude=/dev/* \
  --exclude=/home \
  --exclude=/var/log \
  / /mnt/newroot/@/

# Copy home
if [ -d /home ] && [ "$(ls -A /home 2>/dev/null)" ]; then
  echo "Copying /home..."
  rsync -aAX /home/ /mnt/newroot/@home/
fi

# Copy logs
if [ -d /var/log ] && [ "$(ls -A /var/log 2>/dev/null)" ]; then
  echo "Copying /var/log..."
  rsync -aAX /var/log/ /mnt/newroot/@logs/
fi

# Create directories for other subvolumes
mkdir -p /mnt/newroot/@/var/lib/postgresql
mkdir -p /mnt/newroot/@/var/lib/containers

# Set @ as default subvolume
echo "Setting default subvolume..."
DEFAULT_ID=$(btrfs subvolume list /mnt/newroot | grep '@$' | awk '{print $2}')
btrfs subvolume set-default $DEFAULT_ID /mnt/newroot

# Get UUIDs
ROOT_UUID=$(blkid -s UUID -o value $ROOT_PART)
BOOT_UUID=$(blkid -s UUID -o value $BOOT_PART)
EFI_UUID=$(blkid -s UUID -o value $EFI_PART)
STAGING_UUID=$(blkid -s UUID -o value $STAGING_PART)

echo "UUIDs:"
echo "  Root: $ROOT_UUID"
echo "  Boot: $BOOT_UUID"
echo "  EFI: $EFI_UUID"
echo "  Staging: $STAGING_UUID"

# Create fstab in new root
echo "Creating /etc/fstab..."
cat > /mnt/newroot/@/etc/fstab << EOF
# /etc/fstab: static file system information
UUID=$ROOT_UUID /                       btrfs subvol=@,compress=zstd:6 0 1
UUID=$ROOT_UUID /home                   btrfs subvol=@home,compress=zstd:6 0 2
UUID=$ROOT_UUID /var/log                btrfs subvol=@logs,compress=zstd:6 0 2
UUID=$ROOT_UUID /var/lib/postgresql     btrfs subvol=@postgres,compress=zstd:6 0 2
UUID=$ROOT_UUID /var/lib/containers     btrfs subvol=@containers,compress=zstd:6 0 2
UUID=$BOOT_UUID /boot                   ext4 defaults 0 2
UUID=$EFI_UUID /boot/efi                vfat umask=0077 0 1
/dev/mapper/swap none                   swap sw 0 0
EOF

# Setup encrypted swap configuration
echo "Configuring encrypted swap..."
cat > /mnt/newroot/@/etc/crypttab << EOF
# /etc/crypttab: mappings for encrypted partitions
swap UUID=$STAGING_UUID /dev/urandom swap,cipher=aes-xts-plain64,size=256
EOF

# Update grub to boot from new root
echo "Updating bootloader..."
mount --bind /dev /mnt/newroot/@/dev
mount --bind /proc /mnt/newroot/@/proc
mount --bind /sys /mnt/newroot/@/sys
mount $BOOT_PART /mnt/newroot/@/boot
mount $EFI_PART /mnt/newroot/@/boot/efi

# Update initramfs and grub
echo "Running update-initramfs..."
chroot /mnt/newroot/@ update-initramfs -u -k all

echo "Running update-grub..."
chroot /mnt/newroot/@ update-grub

echo "Installing GRUB..."
chroot /mnt/newroot/@ grub-install --target=x86_64-efi --efi-directory=/boot/efi --bootloader-id=ubuntu --recheck || echo "WARNING: grub-install failed"

# Unmount everything
echo "Unmounting filesystems..."
umount /mnt/newroot/@/boot/efi || true
umount /mnt/newroot/@/boot || true
umount /mnt/newroot/@/sys || true
umount /mnt/newroot/@/proc || true
umount /mnt/newroot/@/dev || true
umount /mnt/newroot || true
rmdir /mnt/newroot || true

echo "Migration complete! System will boot from BTRFS root with encrypted swap."
