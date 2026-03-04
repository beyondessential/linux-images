#!/bin/bash
# This script is copied into /tmp/ and executed via:
#   chroot "$MNT" /bin/bash /tmp/configure.sh "$ARCH" "$VARIANT" "$GRUB_TARGET"
#
# It expects the following to be available under /tmp/:
#   /tmp/packages.sh        — package list (sourced as bash)
#   /tmp/scripts/           — setup scripts (firewall, tailscale, snapper, etc.)
#   /tmp/files/             — static files to install
set -euo pipefail

ARCH="$1"
VARIANT="$2"
GRUB_TARGET="$3"

export DEBIAN_FRONTEND=noninteractive
# minbase doesn't include /usr/sbin in PATH, but that's where locale-gen,
# update-locale, and other admin tools live.
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
# Prevent the host's locale from leaking in and confusing perl/dbus/etc.
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
export LANGUAGE=

echo "--- configure.sh: arch=$ARCH variant=$VARIANT grub_target=$GRUB_TARGET ---"

# ============================================================
# Apt sources
# ============================================================
# Ubuntu 24.04 uses DEB822 format
if [ "$ARCH" = "arm64" ]; then
    MIRROR="http://ports.ubuntu.com/ubuntu-ports"
else
    MIRROR="http://archive.ubuntu.com/ubuntu"
fi

cat > /etc/apt/sources.list.d/ubuntu.sources << EOF
Types: deb
URIs: $MIRROR
Suites: noble noble-updates noble-backports
Components: main restricted universe
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: $MIRROR
Suites: noble-security
Components: main restricted universe
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF

# Remove the debootstrap-generated sources.list if present
rm -f /etc/apt/sources.list

apt-get update -q

# ============================================================
# Bootstrap essential packages
# ============================================================
# minbase is very minimal — we need systemd, a kernel, and dbus before
# we can install the rest of the package list.
apt-get install -y -q --no-install-recommends \
    systemd \
    systemd-sysv \
    dbus \
    sudo \
    locales \
    linux-generic

# Generate all English locales
sed -i '/^# *en_.*UTF-8/s/^# *//' /etc/locale.gen
locale-gen
update-locale LANG=en_US.UTF-8

# Set timezone
ln -sf /usr/share/zoneinfo/Etc/UTC /etc/localtime
echo "Etc/UTC" > /etc/timezone

# ============================================================
# Third-party APT repositories
# ============================================================
# r[image.packages.bes-tools] r[image.packages.tailscale] r[image.packages.kopia]
bash /tmp/scripts/setup-bes-tools.sh
bash /tmp/scripts/setup-kopia.sh

# ============================================================
# Install packages from list
# ============================================================
# r[image.packages.install]: Install all via apt inside the chroot.
source packages.sh
if [ "${#PACKAGES[@]}" -gt 0 ]; then
    echo "Installing ${#PACKAGES[@]} packages..."
    apt-get install -y -q --no-install-recommends "${PACKAGES[@]}"
fi

# ============================================================
# Dracut (replaces initramfs-tools)
# ============================================================
# r[image.boot.dracut]
apt-get install -y -q dracut  # this removes initramfs-tools

install -m 644 /tmp/files/dracut/01-fix-hostonly-noble.conf \
    /etc/dracut.conf.d/01-fix-hostonly-noble.conf

# ============================================================
# Variant identification
# ============================================================
# r[image.variant.types]
mkdir -p /etc/bes
echo "$VARIANT" > /etc/bes/image-variant

# ============================================================
# GRUB configuration
# ============================================================
# Ensure /etc/default/grub exists (grub package should create it)
if [ ! -f /etc/default/grub ]; then
    mkdir -p /etc/default
    cat > /etc/default/grub << 'GRUBEOF'
GRUB_DEFAULT=0
GRUB_TIMEOUT=5
GRUB_TIMEOUT_STYLE=menu
GRUB_DISTRIBUTOR="Ubuntu"
GRUB_CMDLINE_LINUX_DEFAULT="noresume"
GRUB_CMDLINE_LINUX=""
GRUB_RECORDFAIL_TIMEOUT=5
GRUBEOF
else
    # r[image.boot.grub-timeout]
    sed -i 's/^GRUB_TIMEOUT=.*/GRUB_TIMEOUT=5/' /etc/default/grub
    sed -i 's/^GRUB_TIMEOUT_STYLE=.*/GRUB_TIMEOUT_STYLE=menu/' /etc/default/grub

    # r[image.boot.grub-cmdline]
    sed -i 's/^GRUB_CMDLINE_LINUX_DEFAULT=.*/GRUB_CMDLINE_LINUX_DEFAULT="noresume"/' /etc/default/grub

    # r[image.boot.grub-timeout] (recordfail)
    if ! grep -q '^GRUB_RECORDFAIL_TIMEOUT=' /etc/default/grub; then
        echo 'GRUB_RECORDFAIL_TIMEOUT=5' >> /etc/default/grub
    else
        sed -i 's/^GRUB_RECORDFAIL_TIMEOUT=.*/GRUB_RECORDFAIL_TIMEOUT=5/' /etc/default/grub
    fi
fi

# ============================================================
# fstab and crypttab
# ============================================================
if [ "$VARIANT" = "metal" ]; then
    # r[image.luks.keyfile]
    mkdir -p /etc/luks
    touch /etc/luks/empty-keyfile
    chmod 000 /etc/luks/empty-keyfile

    install -m 644 /tmp/files/dracut/02-luks-keyfile.conf \
        /etc/dracut.conf.d/02-luks-keyfile.conf

    # r[image.luks.crypttab]
    cat > /etc/crypttab << 'EOF'
# <name> <device>                    <keyfile>                 <options>
root     /dev/disk/by-partlabel/root /etc/luks/empty-keyfile  force,luks,discard,headless=true,try-empty-password=true
EOF

    cat > /etc/fstab << 'EOF'
# <device>                   <mountpoint>         <fs>  <options>                           <dump> <pass>
/dev/mapper/root             /                    btrfs subvol=@,compress=zstd:6                 0 1
/dev/mapper/root             /var/lib/postgresql   btrfs subvol=@postgres,compress=zstd:6         0 2
/dev/disk/by-partlabel/xboot /boot                ext4  defaults                                 0 2
/dev/disk/by-partlabel/efi   /boot/efi            vfat  umask=0077                               0 1
EOF
else
    cat > /etc/fstab << 'EOF'
# <device>                   <mountpoint>         <fs>  <options>                           <dump> <pass>
/dev/disk/by-partlabel/root  /                    btrfs subvol=@,compress=zstd:6                 0 1
/dev/disk/by-partlabel/root  /var/lib/postgresql   btrfs subvol=@postgres,compress=zstd:6         0 2
/dev/disk/by-partlabel/xboot /boot                ext4  defaults                                 0 2
/dev/disk/by-partlabel/efi   /boot/efi            vfat  umask=0077                               0 1
EOF

    # Ensure no crypttab exists
    rm -f /etc/crypttab
fi

# ============================================================
# Firewall
# ============================================================
# r[image.firewall.policy] r[image.firewall.ssh] r[image.firewall.http] r[image.firewall.enabled]
bash /tmp/scripts/setup-firewall.sh

# ============================================================
# Tailscale
# ============================================================
# r[image.tailscale.service-enabled] r[image.tailscale.ts-up]
bash /tmp/scripts/setup-tailscale.sh

# r[image.tailscale.ts-up]
install -m 755 /tmp/files/ts-up /usr/local/bin/ts-up

# ============================================================
# SSH
# ============================================================
# r[image.credentials.ssh-keys-only]
mkdir -p /etc/ssh/sshd_config.d
cat > /etc/ssh/sshd_config.d/50-bes-no-password.conf << 'EOF'
PasswordAuthentication no
EOF
systemctl enable ssh

# ============================================================
# Snapper
# ============================================================
# r[image.snapper.root] r[image.snapper.postgres] r[image.snapper.timers]
bash /tmp/scripts/setup-snapper.sh

# ============================================================
# Disk growth service
# ============================================================
# r[image.growth.service] r[image.growth.script]
install -m 755 /tmp/files/grow-root-filesystem /usr/local/bin/grow-root-filesystem
install -m 644 /tmp/files/systemd/grow-root-filesystem.service /etc/systemd/system/grow-root-filesystem.service
systemctl enable grow-root-filesystem.service

# ============================================================
# Metal-variant encryption services
# ============================================================
if [ "$VARIANT" = "metal" ]; then
    # r[image.luks.reencrypt]
    install -m 644 /tmp/files/systemd/luks-reencrypt.service /etc/systemd/system/luks-reencrypt.service
    systemctl enable luks-reencrypt.service

    # r[image.tpm.service] r[image.tpm.enrollment]
    install -m 755 /tmp/files/setup-tpm-unlock /usr/local/bin/setup-tpm-unlock
    install -m 644 /tmp/files/systemd/setup-tpm-unlock.service /etc/systemd/system/setup-tpm-unlock.service
    systemctl enable setup-tpm-unlock.service
fi

# ============================================================
# Credentials
# ============================================================
# r[image.credentials.ubuntu-user]
if ! id -u ubuntu &>/dev/null; then
    useradd -m -s /bin/bash -G sudo ubuntu
fi
echo "ubuntu:bes" | chpasswd
passwd --expire ubuntu

# r[image.credentials.root-disabled]
usermod -s /sbin/nologin root

# ============================================================
# Cloud-init
# ============================================================
# r[image.cloud-init.enabled]
# cloud-init is in packages.sh, just configure it.

# r[image.cloud-init.no-hostname-file]
mkdir -p /etc/cloud/cloud.cfg.d
cat > /etc/cloud/cloud.cfg.d/99-bes.cfg << 'EOF'
create_hostname_file: false
ssh_pwauth: false
EOF

# Allow the default user to sudo without password (cloud-init default)
cat > /etc/sudoers.d/90-cloud-init-users << 'EOF'
# Created by BES image builder
ubuntu ALL=(ALL) NOPASSWD:ALL
EOF
chmod 440 /etc/sudoers.d/90-cloud-init-users

# Ensure there's no unminimize message in the MOTD
rm -rvf /mnt/image-root/etc/update-motd.d/60-unminimize

# r[image.cloud-init.no-network]
rm -rvf /mnt/image-root/etc/cloud/cloud.cfg.d/90-installer-network.cfg

# r[image.cloud-init.no-machineid]
truncate -s0 /mnt/image-root/etc/machine-id

# ============================================================
# Generate initramfs and install bootloader
# ============================================================
# Determine kernel version (there should be exactly one)
KVER="$(find /lib/modules -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort -V | tail -n1)"
if [ -z "$KVER" ]; then
    echo "ERROR: no kernel modules found in /lib/modules"
    exit 1
fi
echo "Kernel version: $KVER"

# r[image.boot.dracut]
echo "Generating initramfs with dracut..."
dracut --force --kver "$KVER"

# r[image.boot.grub-install]
echo "Installing GRUB (target=$GRUB_TARGET)..."
rm -rf /boot/grub
mkdir -p /boot/grub
update-grub
grub-install \
    --target="$GRUB_TARGET" \
    --efi-directory=/boot/efi \
    --bootloader-id=ubuntu \
    --no-nvram \
    --removable

# ============================================================
# Hostname (cleared — set by DHCP or cloud-init or installer)
# ============================================================
echo "ubuntu" > /etc/hostname
echo "127.0.0.1 localhost" > /etc/hosts
echo "::1       localhost ip6-localhost ip6-loopback" >> /etc/hosts

# ============================================================
# Final package cleanup
# ============================================================
apt-get autoremove -y -q
apt-get clean

echo "--- configure.sh complete ---"
