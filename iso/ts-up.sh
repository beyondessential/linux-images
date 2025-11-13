#!/bin/bash
set -e

# Tailscale first boot configuration script
# Prompts user to provide Tailscale auth key and connects to network

if [ "$(id -u)" -ne 0 ]; then
  exec sudo "$0" "$@"
fi

if tailscale status > /dev/null; then
  echo "Tailscale is already configured."
  exit
fi

echo ""
echo "=========================================="
echo "Tailscale Configuration"
echo "=========================================="
echo ""
echo "Enter your Tailscale auth key (or press Enter to skip):"
read -r TS_AUTH_KEY

TAILSCALE_CONNECTED=0

if [ -n "$TS_AUTH_KEY" ]; then
  echo "Connecting to Tailscale with auth key..."
  if tailscale up --auth-key="$TS_AUTH_KEY" --ssh; then
    echo "Tailscale configured successfully!"
    TAILSCALE_CONNECTED=1
  else
    echo "WARNING: Failed to connect to Tailscale"
    echo "You can try again later with: sudo ts-up"
  fi
else
  echo ""
  echo "No auth key provided. Starting interactive login..."
  echo "Scan the QR code below with your Tailscale app:"
  echo ""
  if tailscale up --ssh --qr; then
    echo ""
    echo "Tailscale configured successfully!"
    TAILSCALE_CONNECTED=1
  else
    echo ""
    echo "WARNING: Failed to connect to Tailscale"
    echo "You can try again later with: sudo ts-up"
  fi
fi

# Restrict SSH to LAN and Tailscale if connection was successful
if [ $TAILSCALE_CONNECTED -eq 1 ]; then
  echo ""
  echo "Restricting SSH access to LAN and Tailscale..."

  # Remove general SSH rule
  ufw delete allow 22/tcp

  # Allow SSH from private network ranges (RFC1918)
  ufw allow from 10.0.0.0/8 to any port 22 proto tcp comment 'SSH from LAN'
  ufw allow from 172.16.0.0/12 to any port 22 proto tcp comment 'SSH from LAN'
  ufw allow from 192.168.0.0/16 to any port 22 proto tcp comment 'SSH from LAN'

  # Allow SSH from IPv6 ULA
  ufw allow from fc00::/7 to any port 22 proto tcp comment 'SSH from LAN (IPv6)'

  # Allow SSH from Tailscale interface
  ufw allow in on tailscale0 to any port 22 proto tcp comment 'SSH from Tailscale'

  ufw reload

  echo "SSH access restricted to LAN and Tailscale networks."
fi
