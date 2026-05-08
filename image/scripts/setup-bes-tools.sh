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

# r[image.packages.bes-tools]: Pin bes-tools so the right packages come from
# the right place. `bestool` is always preferred from bes-tools (only
# published there). On noble, the Ubuntu archive ships an outdated podman,
# so bes-tools wins for everything via a wildcard pin. On 26.04+, the Ubuntu
# archive ships a sufficiently recent podman — narrow the pin so only
# bestool is forced from bes-tools.
UBUNTU_SUITE="${UBUNTU_SUITE:-noble}"
if [ "$UBUNTU_SUITE" = "noble" ]; then
    cat > /etc/apt/preferences.d/99-bes-tools << 'EOF'
Package: *
Pin: origin tools.ops.tamanu.io
Pin-Priority: 999
EOF
else
    cat > /etc/apt/preferences.d/99-bes-tools << 'EOF'
Package: bestool
Pin: origin tools.ops.tamanu.io
Pin-Priority: 999

Package: *
Pin: origin tools.ops.tamanu.io
Pin-Priority: 100
EOF
fi

apt-get update -q
# r[image.packages.podman]
# r[image.packages.bestool+2]
apt-get install -y podman bestool
