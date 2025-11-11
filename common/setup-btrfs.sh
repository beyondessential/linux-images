#!/bin/bash
set -euo pipefail

# BTRFS subvolume setup script
# This script creates the custom subvolume layout for our Ubuntu systems
# Can be called from both Packer provisioners and autoinstall late-commands

ROOT_DEVICE="${ROOT_DEVICE:-}"
ROOT_MOUNT="${ROOT_MOUNT:-/mnt}"

if [ -z "$ROOT_DEVICE" ]; then
    echo "ERROR: ROOT_DEVICE environment variable must be set"
    echo "Example: ROOT_DEVICE=/dev/sda1 $0"
    exit 1
fi

echo "Setting up BTRFS subvolumes on $ROOT_DEVICE"

# Mount the BTRFS root
mount "$ROOT_DEVICE" "$ROOT_MOUNT"

# Enable BTRFS features
echo "Enabling BTRFS features (squota)..."
btrfs quota enable "$ROOT_MOUNT"

# Create subvolumes
echo "Creating subvolumes..."
btrfs subvolume create "$ROOT_MOUNT/@"
btrfs subvolume create "$ROOT_MOUNT/@home"
btrfs subvolume create "$ROOT_MOUNT/@logs"
btrfs subvolume create "$ROOT_MOUNT/@postgres"
btrfs subvolume create "$ROOT_MOUNT/@containers"
btrfs subvolume create "$ROOT_MOUNT/snapshots"

# Set @ as the default subvolume
echo "Setting @ as default subvolume..."
btrfs subvolume set-default "$ROOT_MOUNT/@"

# Get the BTRFS filesystem UUID for fstab
FS_UUID=$(blkid -s UUID -o value "$ROOT_DEVICE")

# Unmount the root
umount "$ROOT_MOUNT"

# Remount with proper subvolume structure
echo "Mounting subvolumes..."
mount -o subvol=@,compress=zstd:6 "$ROOT_DEVICE" "$ROOT_MOUNT"

# Create mount points
mkdir -p "$ROOT_MOUNT"/{home,var/log,var/lib/postgresql,var/lib/containers}

# Mount other subvolumes
mount -o subvol=@home,compress=zstd:6 "$ROOT_DEVICE" "$ROOT_MOUNT/home"
mount -o subvol=@logs,compress=zstd:6 "$ROOT_DEVICE" "$ROOT_MOUNT/var/log"
mount -o subvol=@postgres,compress=zstd:6 "$ROOT_DEVICE" "$ROOT_MOUNT/var/lib/postgresql"
mount -o subvol=@containers,compress=zstd:6 "$ROOT_DEVICE" "$ROOT_MOUNT/var/lib/containers"

# Generate fstab entries
echo "Generating fstab entries..."
cat >> "$ROOT_MOUNT/etc/fstab" <<EOF

# BTRFS subvolumes
UUID=$FS_UUID /                       btrfs subvol=@,compress=zstd:6 0 1
UUID=$FS_UUID /home                   btrfs subvol=@home,compress=zstd:6 0 2
UUID=$FS_UUID /var/log                btrfs subvol=@logs,compress=zstd:6 0 2
UUID=$FS_UUID /var/lib/postgresql     btrfs subvol=@postgres,compress=zstd:6 0 2
UUID=$FS_UUID /var/lib/containers     btrfs subvol=@containers,compress=zstd:6 0 2
EOF

echo "BTRFS setup complete"
echo "Subvolumes created: @, @home, @logs, @postgres, @containers, snapshots"
