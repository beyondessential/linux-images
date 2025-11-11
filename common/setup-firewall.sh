#!/bin/bash
set -euo pipefail

# UFW firewall configuration script
# Sets up basic firewall rules for web services and SSH

echo "Configuring UFW firewall..."
set -x

# Reset UFW to clean state
ufw --force reset

# Set default policies
ufw default deny incoming
ufw default allow outgoing
ufw default allow forward

# Allow SSH from anywhere (will be restricted after Tailscale connects)
ufw allow 22/tcp comment 'SSH'

# Allow HTTP/HTTPS
ufw allow 80/tcp comment 'HTTP'
ufw allow 443/tcp comment 'TCP HTTPS'
ufw allow 443/udp comment 'UDP HTTPS (HTTP/3)'

# Enable UFW
ufw --force enable
