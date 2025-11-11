#!/bin/bash
set -e

# Fix partition labels and type UUIDs that subiquity may not set correctly
# This runs before migrate-to-btrfs.sh to ensure partitions are properly configured

echo "Fixing partition labels and type UUIDs..."

# Find the main disk
STAGING_PART=$(findmnt -n -o SOURCE /)
DISK="/dev/$(lsblk -ndo PKNAME "$STAGING_PART")"

if [ -z "$DISK" ] || [ ! -b "$DISK" ]; then
  echo "ERROR: Could not determine disk device"
  exit 1
fi

echo "Working with disk: $DISK"

# Display current partition table
echo "Current partition table:"
sgdisk -p "$DISK" || fdisk -l "$DISK"

# Fix partition 1 (EFI)
# Type: C12A7328-F81F-11D2-BA4B-00A0C93EC93B (EFI System)
# Name: efi
echo "Setting partition 1 (EFI)..."
sgdisk -t 1:C12A7328-F81F-11D2-BA4B-00A0C93EC93B -c 1:efi "$DISK" || true

# Fix partition 2 (Boot)
# Type: BC13C2FF-59E6-4262-A352-B275FD6F7172 (Linux extended boot)
# Name: xboot
echo "Setting partition 2 (Boot)..."
sgdisk -t 2:BC13C2FF-59E6-4262-A352-B275FD6F7172 -c 2:xboot "$DISK" || true

# Fix partition 3 (Staging/Swap)
# Type: 0657FD6D-A4AB-43C4-84E5-0933C84B4F4F (Linux swap)
# Name: swap
echo "Setting partition 3 (Staging/Swap)..."
sgdisk -t 3:0657FD6D-A4AB-43C4-84E5-0933C84B4F4F -c 3:swap "$DISK" || true

# Fix partition 4 (Root)
# Type: 4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709 (Linux root x86-64) for amd64
# Type: B921B045-1DF0-41C3-AF44-4C6F280D3FAE (Linux root ARM64) for arm64
# Name: root
echo "Setting partition 4 (Root)..."
ARCH=$(uname -m)
if [ "$ARCH" = "x86_64" ]; then
  sgdisk -t 4:4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709 -c 4:root "$DISK" || true
elif [ "$ARCH" = "aarch64" ]; then
  sgdisk -t 4:B921B045-1DF0-41C3-AF44-4C6F280D3FAE -c 4:root "$DISK" || true
else
  echo "WARNING: Unknown architecture $ARCH, using generic Linux filesystem type"
  sgdisk -t 4:0FC63DAF-8483-4772-8E79-3D69D8477DE4 -c 4:root "$DISK" || true
fi

# Inform kernel of partition table changes
partprobe "$DISK" || true
sleep 2

# Display updated partition table
echo ""
echo "Updated partition table:"
sgdisk -p "$DISK" || fdisk -l "$DISK"

echo ""
echo "Partition labels and type UUIDs fixed successfully"
