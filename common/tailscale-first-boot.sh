#!/bin/bash
set -e

# Tailscale first boot configuration script
# Prompts user to provide Tailscale auth key and connects to network

echo ""
echo "=========================================="
echo "Tailscale Configuration"
echo "=========================================="
echo ""
echo "Enter your Tailscale auth key (or press Enter to skip):"
read -r TS_AUTH_KEY

if [ -n "$TS_AUTH_KEY" ]; then
  echo "Connecting to Tailscale with auth key..."
  if tailscale up --auth-key="$TS_AUTH_KEY" --ssh; then
    echo "Tailscale configured successfully!"
    touch /etc/tailscale-configured
  else
    echo "WARNING: Failed to connect to Tailscale"
    echo "You can try again later with: tailscale up --ssh"
  fi
else
  echo ""
  echo "No auth key provided. Starting interactive login..."
  echo "Scan the QR code below with your Tailscale app:"
  echo ""
  if tailscale up --ssh --qr; then
    echo ""
    echo "Tailscale configured successfully!"
    touch /etc/tailscale-configured
  else
    echo ""
    echo "WARNING: Failed to connect to Tailscale"
    echo "You can try again later with: tailscale up --ssh"
  fi
fi

# Disable this service after first run
systemctl disable tailscale-first-boot.service
