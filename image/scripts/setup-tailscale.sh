#!/bin/bash
# r[image.packages.tailscale] r[image.tailscale.service-enabled]
#
# Install Tailscale from the official apt repository.
# This runs inside the chroot during image build.
set -euxo pipefail

echo "Installing Tailscale..."

# r[image.packages.tailscale]: Install signing key
if [ -f /tmp/files/tailscale-apt.gpg ]; then
    cp /tmp/files/tailscale-apt.gpg /usr/share/keyrings/tailscale-archive-keyring.gpg
else
    echo "WARNING: /tmp/files/tailscale-apt.gpg not found, downloading from web"
    curl -fsSL https://pkgs.tailscale.com/stable/ubuntu/noble.noarmor.gpg \
        | tee /usr/share/keyrings/tailscale-archive-keyring.gpg >/dev/null
fi

# Add Tailscale apt repository
echo "deb [signed-by=/usr/share/keyrings/tailscale-archive-keyring.gpg] https://pkgs.tailscale.com/stable/ubuntu noble main" \
    | tee /etc/apt/sources.list.d/tailscale.list

# r[image.tailscale.pinned]: Pin the Tailscale repo at priority 900
cat > /etc/apt/preferences.d/99-tailscale << 'EOF'
Package: *
Pin: release o=pkgs.tailscale.com
Pin-Priority: 900
EOF

apt-get update -q
apt-get install -y -q tailscale

# r[image.tailscale.service-enabled]: Enable the daemon but don't join a tailnet
systemctl enable tailscaled

# r[image.tailscale.auto-update]: Weekly cron job to upgrade tailscale
mkdir -p /etc/cron.weekly
cat > /etc/cron.weekly/apt-upgrade-tailscale << 'EOF'
#!/bin/sh
apt install -y tailscale
EOF
chmod 0755 /etc/cron.weekly/apt-upgrade-tailscale
