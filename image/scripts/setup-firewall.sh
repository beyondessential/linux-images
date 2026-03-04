#!/bin/bash
# r[image.firewall.policy] r[image.firewall.ssh] r[image.firewall.http] r[image.firewall.enabled]
#
# UFW firewall configuration for BES images.
# Writes config files directly rather than running `ufw` commands,
# because ufw cannot apply iptables rules inside a chroot.
set -euo pipefail

echo "Configuring UFW firewall (file-based)..."

# r[image.firewall.enabled]: Enable ufw at boot
cat > /etc/ufw/ufw.conf << 'EOF'
ENABLED=yes
LOGLEVEL=low
EOF

# r[image.firewall.policy]: deny incoming, allow outgoing, allow forwarding
sed -i \
    -e 's/^DEFAULT_INPUT_POLICY=.*/DEFAULT_INPUT_POLICY="DROP"/' \
    -e 's/^DEFAULT_OUTPUT_POLICY=.*/DEFAULT_OUTPUT_POLICY="ACCEPT"/' \
    -e 's/^DEFAULT_FORWARD_POLICY=.*/DEFAULT_FORWARD_POLICY="ACCEPT"/' \
    /etc/default/ufw

# r[image.firewall.ssh]: Allow SSH
# r[image.firewall.http]: Allow HTTP, HTTPS (TCP+UDP for HTTP/3)
cat > /etc/ufw/user.rules << 'RULES'
*filter
:ufw-user-input - [0:0]
:ufw-user-output - [0:0]
:ufw-user-forward - [0:0]
:ufw-user-limit - [0:0]
:ufw-user-limit-accept - [0:0]
### RULES ###

### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in comment=SSH
-A ufw-user-input -p tcp --dport 22 -j ACCEPT

### tuple ### allow tcp 80 0.0.0.0/0 any 0.0.0.0/0 in comment=HTTP
-A ufw-user-input -p tcp --dport 80 -j ACCEPT

### tuple ### allow tcp 443 0.0.0.0/0 any 0.0.0.0/0 in comment=TCP_HTTPS
-A ufw-user-input -p tcp --dport 443 -j ACCEPT

### tuple ### allow udp 443 0.0.0.0/0 any 0.0.0.0/0 in comment=UDP_HTTPS__HTTP/3_
-A ufw-user-input -p udp --dport 443 -j ACCEPT

### END RULES ###

### LOGGING ###
-A ufw-user-limit -m limit --limit 3/minute -j LOG --log-prefix "[UFW LIMIT BLOCK] "
-A ufw-user-limit -j REJECT
-A ufw-user-limit-accept -j ACCEPT
### END LOGGING ###
COMMIT
RULES

cat > /etc/ufw/user6.rules << 'RULES'
*filter
:ufw6-user-input - [0:0]
:ufw6-user-output - [0:0]
:ufw6-user-forward - [0:0]
:ufw6-user-limit - [0:0]
:ufw6-user-limit-accept - [0:0]
### RULES ###

### tuple ### allow tcp 22 ::/0 any ::/0 in comment=SSH
-A ufw6-user-input -p tcp --dport 22 -j ACCEPT

### tuple ### allow tcp 80 ::/0 any ::/0 in comment=HTTP
-A ufw6-user-input -p tcp --dport 80 -j ACCEPT

### tuple ### allow tcp 443 ::/0 any ::/0 in comment=TCP_HTTPS
-A ufw6-user-input -p tcp --dport 443 -j ACCEPT

### tuple ### allow udp 443 ::/0 any ::/0 in comment=UDP_HTTPS__HTTP/3_
-A ufw6-user-input -p udp --dport 443 -j ACCEPT

### END RULES ###

### LOGGING ###
-A ufw6-user-limit -m limit --limit 3/minute -j LOG --log-prefix "[UFW LIMIT BLOCK] "
-A ufw6-user-limit -j REJECT
-A ufw6-user-limit-accept -j ACCEPT
### END LOGGING ###
COMMIT
RULES

chmod 640 /etc/ufw/user.rules /etc/ufw/user6.rules

echo "UFW firewall configured."
