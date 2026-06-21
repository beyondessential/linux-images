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
# Modern Ubuntu (>=24.04) uses DEB822 format.
UBUNTU_SUITE="${UBUNTU_SUITE:-noble}"

if [ "$ARCH" = "arm64" ]; then
    MIRROR="http://ports.ubuntu.com/ubuntu-ports"
else
    MIRROR="http://archive.ubuntu.com/ubuntu"
fi

cat > /etc/apt/sources.list.d/ubuntu.sources << EOF
Types: deb
URIs: $MIRROR
Suites: $UBUNTU_SUITE $UBUNTU_SUITE-updates $UBUNTU_SUITE-backports
Components: main restricted universe
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: $MIRROR
Suites: $UBUNTU_SUITE-security
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
# we can install the rest of the package list. The kernel package is
# variant-specific: pi uses linux-raspi.
case "$VARIANT" in
    pi)         BOOTSTRAP_KERNEL="linux-raspi" ;;
    metal|cloud) BOOTSTRAP_KERNEL="linux-generic" ;;
esac
apt-get install -y -q --no-install-recommends \
    systemd \
    systemd-sysv \
    dbus \
    sudo \
    locales \
    "$BOOTSTRAP_KERNEL"

# Generate all English locales
sed -i '/^# *en_.*UTF-8/s/^# *//' /etc/locale.gen
locale-gen
update-locale LANG=en_US.UTF-8

# Set timezone
ln -sf /usr/share/zoneinfo/Etc/UTC /etc/localtime
echo "Etc/UTC" > /etc/timezone

# ============================================================
# Install packages from list
# ============================================================
source /tmp/packages.sh
if [ "${#PACKAGES[@]}" -gt 0 ]; then
    echo "Installing ${#PACKAGES[@]} packages..."
    apt-get install -y -q --no-install-recommends "${PACKAGES[@]}"
fi

# r[impl image.packages.chrony]
# chrony enables itself via its postinst; defensively disable
# systemd-timesyncd in case it was pulled in as a dependency (noble's
# systemd-sysv depends on it). chrony.service has a runtime Conflicts=
# directive against systemd-timesyncd, but leaving both enabled means one
# fails to start at boot — better to disable it deterministically here.
if [ -x /usr/lib/systemd/systemd-timesyncd ] || [ -f /usr/lib/systemd/system/systemd-timesyncd.service ]; then
    systemctl disable systemd-timesyncd.service 2>/dev/null || true
    systemctl mask systemd-timesyncd.service
fi

# ============================================================
# Third-party APT repositories
# ============================================================
bash /tmp/scripts/setup-bes-tools.sh
bash /tmp/scripts/setup-kopia.sh

# ============================================================
# Dracut (replaces initramfs-tools)
# ============================================================
# r[image.boot.dracut]
apt-get install -y -q dracut  # this removes initramfs-tools

# Dracut's default is hostonly=yes (per the dracut.conf manpage on every
# supported suite), which produces an initramfs bound to the build host.
# The shipped image needs to be portable across hardware, so we override:
#
# - On noble, hostonly=no is broken — we keep hostonly=yes + sloppy mode and
#   force-include the hardware/cloud module lists.
# - On 26.04+, hostonly=no works correctly and pulls in all kernel modules,
#   so a single drop-in is enough.
#
# The installer strips the 26.04+ override post-install so the target
# machine's initramfs is hostonly=yes (the default), specialised to its
# actual hardware (see r[installer.write.rebuild-boot-config+9]).
if [ "$VARIANT" = "pi" ]; then
    # The hardware/cloud driver lists are x86-server-leaning (e1000e, ixgbe,
    # etc.) and many of those modules don't exist in linux-raspi. Pi always
    # uses the portable-image config (hostonly=no) regardless of suite, so
    # dracut just bundles whatever linux-raspi ships.
    install -m 644 /tmp/files/dracut/01-portable-image.conf \
        /etc/dracut.conf.d/01-portable-image.conf
elif [ "$UBUNTU_SUITE" = "noble" ]; then
    install -m 644 /tmp/files/dracut/01-fix-hostonly.conf \
        /etc/dracut.conf.d/01-fix-hostonly.conf

    # r[impl image.boot.hardware-drivers+3]
    install -m 644 /tmp/files/dracut/03-hardware-drivers.conf \
        /etc/dracut.conf.d/03-hardware-drivers.conf

    # r[impl image.boot.cloud-drivers+5]
    if [ "$VARIANT" = "cloud" ]; then
        install -m 644 /tmp/files/dracut/04-cloud-drivers.conf \
            /etc/dracut.conf.d/04-cloud-drivers.conf
    fi
else
    # r[impl image.boot.hardware-drivers+3] r[impl image.boot.cloud-drivers+5]
    install -m 644 /tmp/files/dracut/01-portable-image.conf \
        /etc/dracut.conf.d/01-portable-image.conf
fi

if [ "$VARIANT" = "metal" ]; then
    apt-get install -y -q --no-install-recommends linux-firmware
fi

# ============================================================
# Console font
# ============================================================
# r[image.base.console-font]
cat > /etc/default/console-setup << 'EOF'
ACTIVE_CONSOLES="/dev/tty[1-6]"
CHARMAP="UTF-8"
CODESET="guess"
FONTFACE="Fixed"
FONTSIZE="8x16"
VIDEOMODE=
EOF

# ============================================================
# Login banner
# ============================================================
# r[image.base.login-banner]
# agetty resolves \4 / \6 against the live network stack each time the
# login prompt is rendered, so this needs no script or systemd unit to
# stay current — the banner reflects whatever addresses are configured
# at the moment of display.
cat > /etc/issue << 'EOF'
\S \n \l

IPv4: \4
IPv6: \6

EOF

# ============================================================
# Variant identification
# ============================================================
# r[image.variant.types+3]
mkdir -p /etc/bes
echo "$VARIANT" > /etc/bes/image-variant

# ============================================================
# Bootloader configuration (GRUB for metal/cloud, Pi firmware for pi)
# ============================================================
if [ "$VARIANT" = "pi" ]; then
    # r[image.boot.pi-firmware] r[image.boot.pi-cmdline] r[image.boot.pi-uart] r[image.boot.pi-pcie-gen3]
    # The Pi 5 EEPROM reads config.txt from /boot/firmware; kernel/initrd/DTB
    # for the active slot live under /boot/firmware/current/ (per the A/B
    # tryboot layout — see r[image.boot.pi-tryboot-rollback]). The shipped
    # config.txt is already in its post-migration form (os_prefix=current/
    # under [all], os_prefix=new/ under [tryboot]); we populate current/
    # ourselves below, after dracut has produced the final initramfs.
    # serial0,115200 is the Pi 5 PL011 UART (mapped via enable_uart=1 in
    # config.txt). It comes last so systemd's serial-getty starts there for
    # login, while kernel boot messages still mirror to tty1 (HDMI) for the
    # rare case a screen is attached.
    mkdir -p /boot/firmware
    install -m 644 /tmp/files/pi/config.txt /boot/firmware/config.txt
    cat > /boot/firmware/cmdline.txt << 'EOF'
console=tty1 console=serial0,115200 root=/dev/mapper/root rootflags=subvol=@,compress=zstd:6 rootfstype=btrfs ro noresume rootwait
EOF

    # r[image.boot.pi-firmware-update] r[image.boot.pi-tryboot-rollback]
    # flash-kernel-piboot (transitional) pulls in piboot-try, which provides
    # /usr/sbin/flash-kernel with Method: pi-try for the Pi 5B, the
    # piboot-try-reboot / piboot-try-validate services, and the kernel
    # postinst hook /etc/kernel/postinst.d/zz-flash-kernel that stages
    # future kernel upgrades into /boot/firmware/new/ on the running device.
    # We do NOT invoke flash-kernel during the build: its main() bails out
    # inside a chroot (systemd-detect-virt --chroot), and its migrate() copies
    # whatever sits at /boot/firmware/ root into current/ — useless for a
    # fresh image. Instead we lay out the A/B directories ourselves further
    # down, after dracut has produced the final initramfs.
    apt-get install -y -q --no-install-recommends flash-kernel-piboot

    # r[image.boot.pi-power-key]
    # Pin the power-button behaviour to a clean shutdown. Default systemd
    # behaviour matches, but shipping it explicitly makes the contract
    # discoverable for whoever wants to flip it (e.g. to `reboot` or
    # `ignore`).
    mkdir -p /etc/systemd/logind.conf.d
    install -m 644 /tmp/files/pi/50-bes-power.conf /etc/systemd/logind.conf.d/50-bes-power.conf
else
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
    fi

    # r[image.boot.cloud-console]
    if [ "$VARIANT" = "cloud" ]; then
        GRUB_CMDLINE="noresume console=ttyS0,115200n8"
    else
        GRUB_CMDLINE="noresume"
    fi

    # r[image.boot.grub-timeout]
    sed -i 's/^GRUB_TIMEOUT=.*/GRUB_TIMEOUT=5/' /etc/default/grub
    sed -i 's/^GRUB_TIMEOUT_STYLE=.*/GRUB_TIMEOUT_STYLE=menu/' /etc/default/grub

    # r[image.boot.grub-cmdline] r[image.boot.cloud-console]
    sed -i "s/^GRUB_CMDLINE_LINUX_DEFAULT=.*/GRUB_CMDLINE_LINUX_DEFAULT=\"${GRUB_CMDLINE}\"/" /etc/default/grub

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
case "$VARIANT" in
    metal|pi)
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
        ;;
esac

# Boot1 fstab line varies: efi (UEFI) → /boot/efi, firmware (Pi) → /boot/firmware.
if [ "$VARIANT" = "pi" ]; then
    BOOT1_FSTAB_LINE="/dev/disk/by-partlabel/firmware /boot/firmware       vfat  umask=0077                               0 1"
else
    BOOT1_FSTAB_LINE="/dev/disk/by-partlabel/efi      /boot/efi            vfat  umask=0077                               0 1"
fi

case "$VARIANT" in
    metal|pi)
        cat > /etc/fstab << EOF
# <device>                   <mountpoint>         <fs>  <options>                           <dump> <pass>
/dev/mapper/root             /                    btrfs subvol=@,compress=zstd:6                 0 1
/dev/mapper/root             /var/lib/postgresql   btrfs subvol=@postgres,compress=zstd:6         0 2
/dev/disk/by-partlabel/xboot /boot                ext4  defaults                                 0 2
$BOOT1_FSTAB_LINE
EOF
        ;;
    cloud)
        cat > /etc/fstab << EOF
# <device>                   <mountpoint>         <fs>  <options>                           <dump> <pass>
/dev/disk/by-partlabel/root  /                    btrfs subvol=@,compress=zstd:6                 0 1
/dev/disk/by-partlabel/root  /var/lib/postgresql   btrfs subvol=@postgres,compress=zstd:6         0 2
/dev/disk/by-partlabel/xboot /boot                ext4  defaults                                 0 2
$BOOT1_FSTAB_LINE
EOF
        rm -f /etc/crypttab
        ;;
esac

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

# r[image.tailscale.firstboot-auth]
install -m 755 /tmp/files/bes-tailscale-firstboot-auth /usr/local/bin/bes-tailscale-firstboot-auth
install -m 644 /tmp/files/systemd/bes-tailscale-firstboot-auth.service /etc/systemd/system/bes-tailscale-firstboot-auth.service
systemctl enable bes-tailscale-firstboot-auth.service

# r[image.firstboot.script]
install -m 755 /tmp/files/bes-firstboot-script /usr/local/bin/bes-firstboot-script
install -m 644 /tmp/files/systemd/bes-firstboot-script.service /etc/systemd/system/bes-firstboot-script.service
systemctl enable bes-firstboot-script.service

# Ship empty default manifests at both supported locations. With the marker-
# file gate on the service, this prevents anyone with post-boot write access
# to the unencrypted firmware partition from introducing a first-boot script
# on an image that didn't ship with one.
install -m 600 /dev/null /etc/bes/firstboot-script
if [ "$VARIANT" = "pi" ]; then
    install -m 600 /dev/null /boot/firmware/firstboot-script
fi

# ============================================================
# Network
# ============================================================
# r[image.base.network+2]
mkdir -p /etc/netplan
install -m 600 /tmp/files/netplan/01-all-en-dhcp.yaml /etc/netplan/01-all-en-dhcp.yaml

# ============================================================
# SSH
# ============================================================
# r[impl image.credentials.no-root-ssh]
mkdir -p /etc/ssh/sshd_config.d
cat > /etc/ssh/sshd_config.d/50-bes-no-root.conf << 'EOF'
PermitRootLogin no
EOF

# r[impl image.credentials.ssh-password-auth]
if [ "$VARIANT" = "cloud" ]; then
    cat > /etc/ssh/sshd_config.d/50-bes-password-auth.conf << 'EOF'
PasswordAuthentication no
EOF
else
    cat > /etc/ssh/sshd_config.d/50-bes-password-auth.conf << 'EOF'
PasswordAuthentication yes
EOF
fi
systemctl enable ssh

# r[impl image.credentials.host-key-regen]
install -m 644 /tmp/files/systemd/bes-ssh-keygen.service /etc/systemd/system/bes-ssh-keygen.service
systemctl enable bes-ssh-keygen.service

# ============================================================
# Snapper
# ============================================================
# r[image.snapper.root] r[image.snapper.postgres] r[image.snapper.timers]
bash /tmp/scripts/setup-snapper.sh

# ============================================================
# Disk growth service
# ============================================================
# r[impl image.growth.service+3]
install -m 755 /tmp/files/grow-root-filesystem /usr/local/bin/grow-root-filesystem
install -m 644 /tmp/files/systemd/grow-root-filesystem.service /etc/systemd/system/grow-root-filesystem.service
systemctl enable grow-root-filesystem.service

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
if [ "$VARIANT" = "cloud" ]; then
    cat > /etc/cloud/cloud.cfg.d/99-bes.cfg << 'EOF'
create_hostname_file: false
ssh_pwauth: false
EOF
else
    cat > /etc/cloud/cloud.cfg.d/99-bes.cfg << 'EOF'
create_hostname_file: false
EOF
fi

# Allow the default user to sudo without password (cloud-init default)
cat > /etc/sudoers.d/90-cloud-init-users << 'EOF'
# Created by BES image builder
ubuntu ALL=(ALL) NOPASSWD:ALL
EOF
chmod 440 /etc/sudoers.d/90-cloud-init-users

# Ensure there's no unminimize message in the MOTD
rm -rvf /etc/update-motd.d/60-unminimize

# r[image.cloud-init.no-network]
rm -rvf /etc/cloud/cloud.cfg.d/90-installer-network.cfg

# r[image.cloud-init.no-machineid]
: > /etc/machine-id

# ============================================================
# Hostname
# ============================================================
# Set hostname before initramfs generation so dracut does not embed a
# stale build-host hostname into the initramfs.
case "$VARIANT" in
    metal|pi)
        # r[image.hostname.metal-dhcp+2]
        : > /etc/hostname
        echo "127.0.0.1 localhost" > /etc/hosts
        echo "::1       localhost ip6-localhost ip6-loopback" >> /etc/hosts
        ;;
    cloud)
        # r[image.hostname.cloud-default+2]
        echo "ubuntu" > /etc/hostname
        echo "127.0.0.1 localhost" > /etc/hosts
        echo "127.0.1.1 ubuntu" >> /etc/hosts
        echo "::1       localhost ip6-localhost ip6-loopback" >> /etc/hosts
        ;;
esac

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

if [ "$VARIANT" = "pi" ]; then
    # r[image.boot.pi-firmware-update] r[image.boot.pi-tryboot-rollback]
    # Lay out /boot/firmware/current/ with the dracut-generated initramfs,
    # the kernel, the Pi 5 DTB, overlays, and the GPU firmware blobs.
    # current/state=good tells piboot-try-reboot.service at next boot that
    # no tryboot is pending. autoboot.txt enables the tryboot A/B chain
    # for future kernel upgrades, which flash-kernel will then stage into
    # /boot/firmware/new/ via the zz-flash-kernel kernel postinst hook.
    echo "Laying out /boot/firmware/current/ for A/B tryboot..."

    mkdir -p /boot/firmware/current/overlays

    install -m 644 "/boot/vmlinuz-$KVER" /boot/firmware/current/vmlinuz
    install -m 644 "/boot/initrd.img-$KVER" /boot/firmware/current/initrd.img
    install -m 644 /boot/firmware/cmdline.txt /boot/firmware/current/cmdline.txt

    DTB_NAME="bcm2712-rpi-5-b.dtb"
    DTB_DIR=""
    for candidate in \
        "/usr/lib/firmware/${KVER}/device-tree/broadcom" \
        "/lib/firmware/${KVER}/device-tree/broadcom"; do
        if [ -f "${candidate}/${DTB_NAME}" ]; then
            DTB_DIR="$candidate"
            break
        fi
    done
    if [ -z "$DTB_DIR" ]; then
        echo "ERROR: could not locate ${DTB_NAME} for kernel ${KVER}" >&2
        exit 1
    fi
    install -m 644 "${DTB_DIR}/${DTB_NAME}" /boot/firmware/current/

    OVERLAYS_SRC="$(dirname "$DTB_DIR")/overlays"
    if [ -d "$OVERLAYS_SRC" ]; then
        # Pi 5 kernels ship overlays as a flat directory of .dtbo files.
        cp "$OVERLAYS_SRC"/*.dtbo /boot/firmware/current/overlays/
    fi

    # GPU firmware blobs (bootcode.bin, start*.elf, fixup*.dat) live in
    # linux-firmware-raspi. They are needed both at /boot/firmware/ root
    # (read before os_prefix takes effect) and inside current/ so the slot
    # is self-contained — flash-kernel does the same at apt-upgrade time.
    BLOB_SRC=/usr/lib/linux-firmware-raspi
    for blob in "$BLOB_SRC"/bootcode.bin "$BLOB_SRC"/start*.elf "$BLOB_SRC"/fixup*.dat; do
        [ -f "$blob" ] || continue
        install -m 644 "$blob" /boot/firmware/
        install -m 644 "$blob" /boot/firmware/current/
    done

    echo good > /boot/firmware/current/state

    cat > /boot/firmware/autoboot.txt << 'EOF'
[all]
tryboot_a_b=1
EOF
else
    # r[image.boot.grub-install] r[image.boot.grub-uuids]
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
fi

# ============================================================
# Final package cleanup
# ============================================================
apt-get autoremove -y -q
apt-get clean

echo "--- configure.sh complete ---"
