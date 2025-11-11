#!/bin/bash
set -euo pipefail

# Tailscale installation script
# Matches the setup in ansible/roles/tailscale

echo "Installing Tailscale..."

# Check if local .deb package is available (from ISO or provisioner)
if [ -f /tmp/tailscale.deb ]; then
  echo "Installing Tailscale from local package..."
  dpkg -i /tmp/tailscale.deb || apt-get install -f -y
else
  echo "Installing Tailscale from repository..."

  # Copy Tailscale GPG key from repository
  # This should be copied to /tmp/tailscale-apt.gpg by the provisioner
  if [ -f /tmp/tailscale-apt.gpg ]; then
    cp /tmp/tailscale-apt.gpg /usr/share/keyrings/tailscale-archive-keyring.gpg
  else
    echo "WARNING: /tmp/tailscale-apt.gpg not found, downloading from web"
    curl -fsSL https://pkgs.tailscale.com/stable/ubuntu/noble.noarmor.gpg | tee /usr/share/keyrings/tailscale-archive-keyring.gpg >/dev/null
  fi

  # Add Tailscale repository
  echo "deb [signed-by=/usr/share/keyrings/tailscale-archive-keyring.gpg] https://pkgs.tailscale.com/stable/ubuntu noble main" | tee /etc/apt/sources.list.d/tailscale.list

  # Prioritise tailscale repo
  cat > /etc/apt/preferences.d/99-tailscale << 'EOF'
Package: *
Pin: release o=pkgs.tailscale.com
Pin-Priority: 900
EOF

  # Update package cache
  apt-get update

  # Install tailscale
  apt-get install -y tailscale
fi

# Enable and start tailscaled service
systemctl enable tailscaled
systemctl start tailscaled

# Set up auto-upgrade via cron
cat > /etc/cron.weekly/apt-upgrade-tailscale << 'EOF'
#!/bin/sh
apt install -y tailscale
EOF
chmod 0755 /etc/cron.weekly/apt-upgrade-tailscale

echo "Tailscale installed successfully"
echo "To connect to Tailscale network, run: tailscale up"
