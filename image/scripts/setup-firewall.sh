#!/bin/bash
# r[image.firewall.policy] r[image.firewall.ssh] r[image.firewall.http] r[image.firewall.enabled]
# r[image.firewall.manageable]
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

# r[image.firewall.manageable]: ufw stores each rule's comment hex-encoded in
# its tuple line; a plain-text comment there makes `ufw status` crash.
hex() {
    printf '%s' "$1" | od -An -tx1 | tr -d ' \n'
}

# Emits one inbound allow rule: ufw's tuple metadata line, then the iptables
# rule it describes.
allow_rule() {
    local prefix="$1" any="$2" proto="$3" port="$4" comment="$5"
    printf '### tuple ### allow %s %s %s any %s in comment=%s\n' \
        "$proto" "$port" "$any" "$any" "$(hex "$comment")"
    printf -- '-A %s-user-input -p %s --dport %s -j ACCEPT\n\n' \
        "$prefix" "$proto" "$port"
}

write_rules() {
    local prefix="$1" any="$2"
    cat << EOF
*filter
:${prefix}-user-input - [0:0]
:${prefix}-user-output - [0:0]
:${prefix}-user-forward - [0:0]
:${prefix}-user-limit - [0:0]
:${prefix}-user-limit-accept - [0:0]
### RULES ###

EOF
    # r[image.firewall.ssh]: Allow SSH
    allow_rule "$prefix" "$any" tcp 22 'SSH'
    # r[image.firewall.http]: Allow HTTP, HTTPS (TCP+UDP for HTTP/3)
    allow_rule "$prefix" "$any" tcp 80 'HTTP'
    allow_rule "$prefix" "$any" tcp 443 'HTTPS'
    allow_rule "$prefix" "$any" udp 443 'HTTP/3'
    cat << EOF
### END RULES ###

### LOGGING ###
-A ${prefix}-user-limit -m limit --limit 3/minute -j LOG --log-prefix "[UFW LIMIT BLOCK] "
-A ${prefix}-user-limit -j REJECT
-A ${prefix}-user-limit-accept -j ACCEPT
### END LOGGING ###
COMMIT
EOF
}

write_rules ufw 0.0.0.0/0 > /etc/ufw/user.rules
write_rules ufw6 ::/0 > /etc/ufw/user6.rules

chmod 640 /etc/ufw/user.rules /etc/ufw/user6.rules

echo "UFW firewall configured."
