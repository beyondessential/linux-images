#!/bin/bash
# Configure the BES tools APT repository.
# This runs inside the chroot during image build.
set -euxo pipefail

echo "Configuring bes-tools APT repository..."

# Install signing key
curl -fsSL https://tools.ops.tamanu.io/apt/bes-tools.gpg.key \
    | gpg --dearmor -o /etc/apt/keyrings/bes-tools.gpg

# Add bes-tools apt repository
# r[image.packages.bes-tools]
echo "deb [signed-by=/etc/apt/keyrings/bes-tools.gpg] https://tools.ops.tamanu.io/apt stable main" \
    > /etc/apt/sources.list.d/bes-tools.list

# r[image.packages.bes-tools]: Pin the bes-tools repo at priority 999
cat > /etc/apt/preferences.d/99-bes-tools << 'EOF'
Package: *
Pin: origin tools.ops.tamanu.io
Pin-Priority: 999
EOF

apt-get update -q
apt-get install -y \
    # r[image.packages.caddy]
    caddy \
    # r[image.packages.podman]
    podman \
    # r[image.packages.bestool]
    bestool
