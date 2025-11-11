#!/bin/bash
# Matches the setup in ansible/roles/tailscale
echo "Installing Tailscale..."
set -euxo pipefail

if [ -f /tmp/tailscale-apt.gpg ]; then
  cp /tmp/tailscale-apt.gpg /usr/share/keyrings/tailscale-archive-keyring.gpg
else
  echo "WARNING: /tmp/tailscale-apt.gpg not found, downloading from web"
  curl -fsSL https://pkgs.tailscale.com/stable/ubuntu/noble.noarmor.gpg | tee /usr/share/keyrings/tailscale-archive-keyring.gpg >/dev/null
fi

echo "deb [signed-by=/usr/share/keyrings/tailscale-archive-keyring.gpg] https://pkgs.tailscale.com/stable/ubuntu noble main" | tee /etc/apt/sources.list.d/tailscale.list

cat > /etc/apt/preferences.d/99-tailscale << 'EOF'
Package: *
Pin: release o=pkgs.tailscale.com
Pin-Priority: 900
EOF

apt-get update
apt-get install -y tailscale

systemctl enable tailscaled

mkdir /etc/cron.weekly
cat > /etc/cron.weekly/apt-upgrade-tailscale << 'EOF'
#!/bin/sh
apt install -y tailscale
EOF
chmod 0755 /etc/cron.weekly/apt-upgrade-tailscale
