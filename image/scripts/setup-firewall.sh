#!/bin/bash
# r[image.firewall.policy] r[image.firewall.ssh] r[image.firewall.http] r[image.firewall.enabled]
#
# UFW firewall configuration for BES images.
# Sets default policies and opens ports for SSH and web traffic.
set -euo pipefail

echo "Configuring UFW firewall..."
set -x

# Reset to clean state
ufw --force reset

# r[image.firewall.policy]: deny incoming, allow outgoing, allow forwarding
ufw default deny incoming
ufw default allow outgoing
ufw default allow forward

# r[image.firewall.ssh]: Allow SSH from anywhere (restricted later when Tailscale connects)
ufw allow 22/tcp comment 'SSH'

# r[image.firewall.http]: Allow HTTP, HTTPS, and HTTP/3
ufw allow 80/tcp comment 'HTTP'
ufw allow 443/tcp comment 'TCP HTTPS'
ufw allow 443/udp comment 'UDP HTTPS (HTTP/3)'

# r[image.firewall.enabled]
ufw --force enable
