#!/bin/bash
set -euo pipefail

# UFW firewall configuration script
# Sets up basic firewall rules for web services and SSH

echo "Configuring UFW firewall..."

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

# Allow ICMP (ping) for IPv4 and IPv6
ufw allow proto icmp comment 'ICMP ping'
ufw allow proto ipv6-icmp comment 'ICMPv6 ping'

# Enable UFW
ufw --force enable

echo "UFW firewall configured"
echo "Rules:"
echo "  - SSH (22/tcp): allowed from anywhere"
echo "  - HTTP (80/tcp): allowed"
echo "  - HTTPS (443/tcp+udp): allowed"
echo "  - ICMP ping: allowed"
