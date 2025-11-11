#!/bin/bash
set -euxo pipefail

# Common provisioning script for all image types
# Runs on both QEMU and AWS builders

echo "=== Starting common provisioning ==="

# Update package cache
sudo apt-get update

# Install base packages from packages.txt
echo "=== Installing base packages ==="
grep -v '^#' /tmp/packages.txt | grep -v '^$' | while read -r package; do
    sudo apt-get install -y "$package" || echo "Warning: Failed to install $package"
done

# Ensure BTRFS tools are installed
sudo apt-get install -y btrfs-progs cryptsetup

# Configure system timezone
sudo timedatectl set-timezone UTC

# Enable automatic security updates
sudo apt-get install -y unattended-upgrades
sudo dpkg-reconfigure -plow unattended-upgrades

# Configure firewall
echo "=== Configuring firewall ==="
sudo bash /tmp/setup-firewall.sh

# Install Tailscale
echo "=== Installing Tailscale ==="
sudo bash /tmp/setup-tailscale.sh

# Configure SSH
echo "=== Configuring SSH ==="
sudo sed -i 's/#PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config
sudo sed -i 's/#PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
sudo sed -i 's/#PubkeyAuthentication.*/PubkeyAuthentication yes/' /etc/ssh/sshd_config

# Enable and configure UFW (firewall)
echo "=== Configuring firewall ==="
sudo ufw --force enable
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow ssh

# Configure systemd journal
echo "=== Configuring systemd journal ==="
sudo mkdir -p /etc/systemd/journald.conf.d
cat <<EOF | sudo tee /etc/systemd/journald.conf.d/99-custom.conf
[Journal]
Storage=persistent
Compress=yes
SystemMaxUse=500M
SystemMaxFileSize=50M
MaxRetentionSec=1month
EOF

# Set up BTRFS maintenance timer for scrubbing
echo "=== Setting up BTRFS maintenance ==="
cat <<EOF | sudo tee /etc/systemd/system/btrfs-scrub.service
[Unit]
Description=BTRFS scrub on root filesystem
After=local-fs.target

[Service]
Type=oneshot
ExecStart=/usr/bin/btrfs scrub start -B /
Nice=19
IOSchedulingClass=idle
EOF

cat <<EOF | sudo tee /etc/systemd/system/btrfs-scrub.timer
[Unit]
Description=Monthly BTRFS scrub

[Timer]
OnCalendar=monthly
Persistent=true

[Install]
WantedBy=timers.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable btrfs-scrub.timer

# Create snapshot directory structure
echo "=== Creating snapshot directory structure ==="
sudo mkdir -p /snapshots

# Install snapshot management script
cat <<'EOF' | sudo tee /usr/local/bin/btrfs-snapshot
#!/bin/bash
set -euo pipefail

SNAPSHOT_NAME="${1:-manual}"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
SNAPSHOT_PATH="/snapshots/@${SNAPSHOT_NAME}/${TIMESTAMP}"

echo "Creating snapshot: $SNAPSHOT_PATH"
btrfs subvolume snapshot -r / "$SNAPSHOT_PATH"
echo "Snapshot created successfully"

# Cleanup old snapshots (keep last 5 of each type)
find "/snapshots/@${SNAPSHOT_NAME}" -maxdepth 1 -type d | sort -r | tail -n +6 | while read -r old_snapshot; do
    echo "Removing old snapshot: $old_snapshot"
    btrfs subvolume delete "$old_snapshot"
done
EOF

sudo chmod +x /usr/local/bin/btrfs-snapshot

# Set up automatic snapshots before updates
cat <<EOF | sudo tee /etc/apt/apt.conf.d/80-btrfs-snapshot
DPkg::Pre-Invoke {"/usr/local/bin/btrfs-snapshot apt-update || true";};
EOF

# Configure kernel parameters for BTRFS
echo "=== Configuring kernel parameters ==="
cat <<EOF | sudo tee -a /etc/sysctl.d/99-btrfs.conf
# BTRFS optimizations
vm.swappiness=10
vm.vfs_cache_pressure=50
EOF

# Disable unnecessary services
echo "=== Disabling unnecessary services ==="
sudo systemctl disable snapd.service || true
sudo systemctl disable snapd.socket || true

# Configure locale
sudo locale-gen en_US.UTF-8
sudo update-locale LANG=en_US.UTF-8

# Set hostname template
echo "ubuntu-custom" | sudo tee /etc/hostname

echo "=== Common provisioning complete ==="
