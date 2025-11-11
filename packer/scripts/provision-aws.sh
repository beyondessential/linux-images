#!/bin/bash
set -euxo pipefail

# AWS-specific provisioning script
# Only runs on amazon-ebs builders

echo "=== Starting AWS-specific provisioning ==="

# Install and configure cloud-init for AWS
echo "=== Configuring cloud-init for AWS ==="
sudo apt-get install -y cloud-init cloud-initramfs-growroot

# Configure cloud-init datasource for AWS
cat <<EOF | sudo tee /etc/cloud/cloud.cfg.d/90-aws.cfg
datasource_list: [ Ec2, None ]
datasource:
  Ec2:
    strict_id: false
    timeout: 5
    max_wait: 10
EOF

# Enable required cloud-init modules
cat <<EOF | sudo tee /etc/cloud/cloud.cfg.d/91-custom.cfg
# Custom cloud-init configuration
system_info:
  distro: ubuntu
  default_user:
    name: ubuntu
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    lock_passwd: true

# Enable growth of root filesystem
growpart:
  mode: auto
  devices: ['/']

resize_rootfs: true

# Enable SSH key injection
ssh_pwauth: false
ssh_authorized_keys: []

# Disable snap seeding on first boot
snap:
  commands: []
EOF

# Install AWS Systems Manager agent
echo "=== Installing AWS Systems Manager agent ==="
sudo snap install amazon-ssm-agent --classic || true
sudo systemctl enable snap.amazon-ssm-agent.amazon-ssm-agent.service || true

# Install AWS CLI
echo "=== Installing AWS CLI ==="
sudo apt-get install -y awscli

# Install EC2 instance connect
sudo apt-get install -y ec2-instance-connect

# Configure ENI hotplug for network interface changes
echo "=== Configuring network interface hotplug ==="
cat <<EOF | sudo tee /etc/netplan/60-aws-eni-hotplug.yaml
network:
  version: 2
  ethernets:
    eth0:
      dhcp4: true
      dhcp6: false
EOF

# Enable ENA (Elastic Network Adapter) support
echo "=== Enabling ENA support ==="
sudo modprobe ena || true
echo "ena" | sudo tee -a /etc/modules

# Configure NVMe support for EBS volumes
echo "=== Configuring NVMe support ==="
sudo modprobe nvme || true
echo "nvme" | sudo tee -a /etc/modules

# Install and configure chrony for time synchronization with AWS time service
echo "=== Configuring time synchronization ==="
sudo apt-get install -y chrony
cat <<EOF | sudo tee /etc/chrony/sources.d/aws.sources
# Amazon Time Sync Service
server 169.254.169.123 prefer iburst minpoll 4 maxpoll 4
EOF
sudo systemctl restart chrony

# Configure metadata service access
echo "=== Configuring metadata service ==="
sudo apt-get install -y ec2-instance-identity-document

# Set up IMDSv2 (Instance Metadata Service v2) enforcement
cat <<EOF | sudo tee /etc/profile.d/aws-metadata.sh
# AWS IMDSv2 helper
export AWS_METADATA_TOKEN=\$(curl -X PUT "http://169.254.169.254/latest/api/token" -H "X-aws-ec2-metadata-token-ttl-seconds: 21600" 2>/dev/null || true)
EOF

# Configure serial console access
echo "=== Enabling serial console ==="
sudo systemctl enable serial-getty@ttyS0.service

# Install CloudWatch agent
echo "=== Installing CloudWatch agent ==="
wget -q https://s3.amazonaws.com/amazoncloudwatch-agent/ubuntu/$(dpkg --print-architecture)/latest/amazon-cloudwatch-agent.deb -O /tmp/amazon-cloudwatch-agent.deb
sudo dpkg -i /tmp/amazon-cloudwatch-agent.deb || true
rm -f /tmp/amazon-cloudwatch-agent.deb

# Configure instance store support (if available)
echo "=== Configuring instance store support ==="
cat <<'EOF' | sudo tee /usr/local/bin/setup-instance-store
#!/bin/bash
# Automatically set up instance store volumes if available
for device in /dev/nvme*n1; do
    if [ -b "$device" ] && ! mountpoint -q "$device"; then
        # Check if it's an instance store (not EBS)
        if nvme id-ctrl -v "$device" | grep -q "0000:00:00.0"; then
            mkfs.ext4 -F "$device"
            mkdir -p /mnt/instance-store
            mount "$device" /mnt/instance-store
            chmod 1777 /mnt/instance-store
        fi
    fi
done
EOF
sudo chmod +x /usr/local/bin/setup-instance-store

# Create systemd service for instance store setup
cat <<EOF | sudo tee /etc/systemd/system/setup-instance-store.service
[Unit]
Description=Setup instance store volumes
After=local-fs.target

[Service]
Type=oneshot
ExecStart=/usr/local/bin/setup-instance-store
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable setup-instance-store.service

# Configure GRUB for AWS (serial console, ENA, etc.)
echo "=== Configuring GRUB for AWS ==="
sudo sed -i 's/GRUB_CMDLINE_LINUX_DEFAULT=.*/GRUB_CMDLINE_LINUX_DEFAULT="console=tty0 console=ttyS0,115200n8 nvme_core.io_timeout=4294967295"/' /etc/default/grub
sudo sed -i 's/GRUB_CMDLINE_LINUX=.*/GRUB_CMDLINE_LINUX="net.ifnames=0 biosdevname=0"/' /etc/default/grub
sudo update-grub

# Configure AWS region detection
cat <<'EOF' | sudo tee /usr/local/bin/get-aws-region
#!/bin/bash
TOKEN=$(curl -X PUT "http://169.254.169.254/latest/api/token" -H "X-aws-ec2-metadata-token-ttl-seconds: 21600" 2>/dev/null)
curl -H "X-aws-ec2-metadata-token: $TOKEN" http://169.254.169.254/latest/meta-data/placement/region 2>/dev/null
EOF
sudo chmod +x /usr/local/bin/get-aws-region

# Set up AWS-specific motd
cat <<'EOF' | sudo tee /etc/update-motd.d/60-aws-info
#!/bin/bash
echo ""
echo "AWS Instance Information:"
TOKEN=$(curl -X PUT "http://169.254.169.254/latest/api/token" -H "X-aws-ec2-metadata-token-ttl-seconds: 21600" 2>/dev/null)
if [ -n "$TOKEN" ]; then
    INSTANCE_ID=$(curl -H "X-aws-ec2-metadata-token: $TOKEN" http://169.254.169.254/latest/meta-data/instance-id 2>/dev/null)
    INSTANCE_TYPE=$(curl -H "X-aws-ec2-metadata-token: $TOKEN" http://169.254.169.254/latest/meta-data/instance-type 2>/dev/null)
    AZ=$(curl -H "X-aws-ec2-metadata-token: $TOKEN" http://169.254.169.254/latest/meta-data/placement/availability-zone 2>/dev/null)
    echo "  Instance ID: $INSTANCE_ID"
    echo "  Instance Type: $INSTANCE_TYPE"
    echo "  Availability Zone: $AZ"
fi
echo ""
EOF
sudo chmod +x /etc/update-motd.d/60-aws-info

# Ensure cloud-init will run on first boot
sudo cloud-init clean --logs --seed

echo "=== AWS-specific provisioning complete ==="
