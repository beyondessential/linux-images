#!/bin/bash
set -euxo pipefail

# Common provisioning script for all image types
# Note: For bare metal builds using the custom ISO, most provisioning
# is already done during the autoinstall process. This script mainly
# serves as a verification step.

echo "=== Starting common provisioning ==="

# Verify system is up to date
sudo apt-get update

# Verify expected packages are installed
echo "=== Verifying installed packages ==="
for pkg in btrfs-progs cryptsetup dracut-core openssh-server ufw tailscale; do
    if dpkg -l | grep -q "^ii  $pkg "; then
        echo "✓ $pkg is installed"
    else
        echo "✗ $pkg is NOT installed"
    fi
done

# Verify services are enabled
echo "=== Verifying services ==="
for svc in ssh tailscaled ufw; do
    if systemctl is-enabled $svc >/dev/null 2>&1; then
        echo "✓ $svc is enabled"
    else
        echo "✗ $svc is NOT enabled"
    fi
done

# Display system information
echo "=== System Information ==="
echo "Hostname: $(hostname)"
echo "Kernel: $(uname -r)"
echo "Architecture: $(uname -m)"
echo "Init system: $(readlink /sbin/init)"

# Display filesystem information
echo "=== Filesystem Information ==="
df -h
echo ""
lsblk -f

# Display BTRFS subvolumes if present
if mount | grep -q btrfs; then
    echo "=== BTRFS Subvolumes ==="
    sudo btrfs subvolume list / || true
fi

# Display encryption status if present
if [ -e /dev/mapper/root ]; then
    echo "=== LUKS Encryption Status ==="
    sudo cryptsetup status root || true
fi

echo "=== Common provisioning complete ==="
