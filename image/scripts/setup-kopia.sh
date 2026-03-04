#!/bin/bash
# Install Kopia from the official APT repository.
# This runs inside the chroot during image build.
set -euxo pipefail

echo "Installing Kopia..."

# Install signing key
curl -fsSL https://kopia.io/signing-key \
    | gpg --dearmor -o /etc/apt/keyrings/kopia-keyring.gpg

# Add Kopia apt repository
# r[image.packages.kopia]
echo "deb [signed-by=/etc/apt/keyrings/kopia-keyring.gpg] http://packages.kopia.io/apt/ stable main" \
    > /etc/apt/sources.list.d/kopia.list

# Pin the Kopia repo at priority 900
cat > /etc/apt/preferences.d/99-kopia << 'EOF'
Package: *
Pin: origin packages.kopia.io
Pin-Priority: 900
EOF

apt-get update -q
apt-get install -y \
    # r[image.packages.kopia]
    kopia
