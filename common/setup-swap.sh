#!/bin/bash
set -euo pipefail

# Encrypted swap setup script
# Creates a 4G swap partition encrypted with a random key on each boot
# No hibernation support

SWAP_DEVICE="${SWAP_DEVICE:-}"

if [ -z "$SWAP_DEVICE" ]; then
    echo "ERROR: SWAP_DEVICE environment variable must be set"
    echo "Example: SWAP_DEVICE=/dev/sda2 $0"
    exit 1
fi

echo "Setting up encrypted swap on $SWAP_DEVICE"

# Get the partition UUID
SWAP_UUID=$(blkid -s UUID -o value "$SWAP_DEVICE")

if [ -z "$SWAP_UUID" ]; then
    echo "ERROR: Could not determine UUID of $SWAP_DEVICE"
    exit 1
fi

# Configure crypttab for encrypted swap with random key
echo "Configuring encrypted swap in /etc/crypttab..."
cat >> /etc/crypttab <<EOF

# Encrypted swap with random key (no hibernation support)
swap UUID=$SWAP_UUID /dev/urandom swap,cipher=aes-xts-plain64,size=256
EOF

# Configure fstab for the encrypted swap
echo "Configuring swap in /etc/fstab..."
cat >> /etc/fstab <<EOF

# Encrypted swap
/dev/mapper/swap none swap sw 0 0
EOF

echo "Encrypted swap setup complete"
echo "Swap will be encrypted with a random key on each boot"
echo "WARNING: Hibernation is not supported with this configuration"
