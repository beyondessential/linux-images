#!/bin/bash
#
# Build the live ISO base rootfs: debootstrap, install packages, configure
# the live environment (systemd units, initramfs hooks, networking, etc.),
# and produce a tarball. This is the slow part (~5+ min) and is cached.
#
# The installer binary is NOT included here — it is injected later by
# build-iso-rootfs.sh so that installer-only changes skip this step.
#
# Output: a tarball (OUTPUT) containing the full rootfs tree.
#
# Usage: build-iso-base.sh
#   Environment variables:
#     ARCH          - amd64 or arm64 (default: amd64)
#     OUTPUT        - output tarball path (required)
#     UBUNTU_SUITE  - Ubuntu suite name (default: noble)
#     UBUNTU_MIRROR - mirror URL (auto-selected per arch if unset)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOTFS_FILES="$SCRIPT_DIR/rootfs-files"

ARCH="${ARCH:-amd64}"
UBUNTU_SUITE="${UBUNTU_SUITE:-noble}"
OUTPUT="${OUTPUT:?OUTPUT must be set to the tarball path}"

# r[impl iso.per-arch]
case "$ARCH" in
    amd64)
        UBUNTU_MIRROR="${UBUNTU_MIRROR:-http://archive.ubuntu.com/ubuntu}"
        ;;
    arm64)
        UBUNTU_MIRROR="${UBUNTU_MIRROR:-http://ports.ubuntu.com/ubuntu-ports}"
        ;;
    *)
        echo "ERROR: ARCH must be amd64 or arm64 (got: $ARCH)"
        exit 1
        ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root"
    exit 1
fi

MISSING=()
for cmd in debootstrap; do
    command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
done
if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "ERROR: missing required commands: ${MISSING[*]}"
    exit 1
fi

echo "=============================="
echo "BES Live ISO — Base Builder"
echo "=============================="
echo "Architecture:  $ARCH"
echo "Output:        $OUTPUT"
echo "Suite:         $UBUNTU_SUITE"
echo "=============================="
echo ""

# ============================================================
# State tracking for cleanup
# ============================================================
WORK_DIR=""
MNT_ROOTFS=""
CHROOT_MOUNTS_ACTIVE=0

cleanup() {
    local exit_code=$?
    echo ""
    if [ $exit_code -ne 0 ]; then
        echo "!!! Base build failed (exit code $exit_code), cleaning up..."
    else
        echo "Cleaning up..."
    fi

    set +e

    if [ $CHROOT_MOUNTS_ACTIVE -eq 1 ] && [ -n "$MNT_ROOTFS" ]; then
        umount "$MNT_ROOTFS/dev/pts" 2>/dev/null
        umount "$MNT_ROOTFS/dev"     2>/dev/null
        umount "$MNT_ROOTFS/proc"    2>/dev/null
        umount "$MNT_ROOTFS/sys"     2>/dev/null
        umount "$MNT_ROOTFS/run"     2>/dev/null
    fi

    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi

    if [ $exit_code -ne 0 ]; then
        rm -f "$OUTPUT"
    fi
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-base-XXXXXX)"
MNT_ROOTFS="$WORK_DIR/rootfs"

mkdir -p "$MNT_ROOTFS"

# Helper: run a command inside the chroot with a sane PATH and locale.
run_in_chroot() {
    chroot "$MNT_ROOTFS" /usr/bin/env \
        PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
        LC_ALL=C \
        "$@"
}

# ============================================================
# Phase 1: Build live rootfs via debootstrap
# ============================================================
# r[impl iso.base+2]
echo "==> Phase 1: Building live rootfs (default variant)..."

DEBOOTSTRAP_EXTRA_ARGS=()
if [ ! -f /usr/share/keyrings/ubuntu-archive-keyring.gpg ]; then
    echo "    (Ubuntu keyring not found on host -- using --no-check-gpg)"
    DEBOOTSTRAP_EXTRA_ARGS+=(--no-check-gpg)
fi

debootstrap \
    --arch="$ARCH" \
    "${DEBOOTSTRAP_EXTRA_ARGS[@]}" \
    "$UBUNTU_SUITE" "$MNT_ROOTFS" "$UBUNTU_MIRROR"

# ============================================================
# Phase 2: Install packages in chroot (including live-boot)
# ============================================================
echo "==> Phase 2: Installing live environment packages..."

mount -t proc proc "$MNT_ROOTFS/proc"
mount -t sysfs sysfs "$MNT_ROOTFS/sys"
mount --bind /dev "$MNT_ROOTFS/dev"
mount --bind /dev/pts "$MNT_ROOTFS/dev/pts"
mount -t tmpfs tmpfs "$MNT_ROOTFS/run"
CHROOT_MOUNTS_ACTIVE=1

rm -f "$MNT_ROOTFS/etc/resolv.conf"
if [ -f /etc/resolv.conf ]; then
    cp --dereference /etc/resolv.conf "$MNT_ROOTFS/etc/resolv.conf"
elif [ -f /run/systemd/resolve/stub-resolv.conf ]; then
    cp --dereference /run/systemd/resolve/stub-resolv.conf "$MNT_ROOTFS/etc/resolv.conf"
else
    echo "nameserver 1.1.1.1" > "$MNT_ROOTFS/etc/resolv.conf"
fi

# r[impl iso.minimal+3]
# r[impl iso.live-boot]
# r[impl iso.offline]
# r[impl iso.network-tools+3]
cat > "$MNT_ROOTFS/etc/apt/sources.list.d/universe.list" << SOURCES
deb $UBUNTU_MIRROR $UBUNTU_SUITE main universe
deb $UBUNTU_MIRROR $UBUNTU_SUITE-updates main universe
deb $UBUNTU_MIRROR $UBUNTU_SUITE-security main universe
SOURCES

run_in_chroot bash -c "
    export DEBIAN_FRONTEND=noninteractive

    apt-get update -q

    apt-get install -y -q --no-install-recommends \
        linux-generic \
        linux-firmware \
        initramfs-tools \
        live-boot \
        live-boot-initramfs-tools \
        systemd-sysv \
        parted \
        gdisk \
        cloud-guest-utils \
        zstd \
        cryptsetup \
        tpm2-tools \
        btrfs-progs \
        lvm2 \
        dosfstools \
        mtools \
        pciutils \
        usbutils \
        curl

    curl -fsSL https://pkgs.tailscale.com/stable/ubuntu/${UBUNTU_SUITE}.noarmor.gpg \
        -o /usr/share/keyrings/tailscale-archive-keyring.gpg
    curl -fsSL https://pkgs.tailscale.com/stable/ubuntu/${UBUNTU_SUITE}.tailscale-keyring.list \
        -o /etc/apt/sources.list.d/tailscale.list
    apt-get update -q
    apt-get install -y -q --no-install-recommends tailscale

    apt-get clean
    rm -rf /var/lib/apt/lists/*
"

# r[impl iso.verity.initramfs-hook]
install -D -m 755 "$SCRIPT_DIR/initramfs/hooks/verity" \
    "$MNT_ROOTFS/usr/share/initramfs-tools/hooks/verity"
install -D -m 755 "$SCRIPT_DIR/initramfs/scripts/live-premount/verity" \
    "$MNT_ROOTFS/usr/share/initramfs-tools/scripts/live-premount/verity"

echo "    Rebuilding initramfs to include verity hook..."
run_in_chroot update-initramfs -u -k all

# ============================================================
# Phase 3: OS-level configuration
# ============================================================
echo "==> Phase 3: Configuring live environment..."

# r[impl iso.network-config+2]
install -D -m 600 "$ROOTFS_FILES/etc/netplan/01-all-en-dhcp.yaml" \
    "$MNT_ROOTFS/etc/netplan/01-all-en-dhcp.yaml"

mkdir -p "$MNT_ROOTFS/etc/network"

# r[impl iso.blacklist-drm]
install -D -m 644 "$ROOTFS_FILES/etc/modprobe.d/blacklist-gpu.conf" \
    "$MNT_ROOTFS/etc/modprobe.d/blacklist-gpu.conf"

# r[impl iso.boot.autostart+3]
install -D -m 755 "$ROOTFS_FILES/usr/local/bin/bes-installer-wrapper" \
    "$MNT_ROOTFS/usr/local/bin/bes-installer-wrapper"

install -D -m 644 "$ROOTFS_FILES/etc/systemd/system/bes-chvt.service" \
    "$MNT_ROOTFS/etc/systemd/system/bes-chvt.service"

install -D -m 644 "$ROOTFS_FILES/etc/systemd/system/bes-installer.service" \
    "$MNT_ROOTFS/etc/systemd/system/bes-installer.service"

run_in_chroot systemctl enable bes-chvt.service
run_in_chroot systemctl enable bes-installer.service

run_in_chroot systemctl mask getty@tty2.service
run_in_chroot systemctl mask autovt@tty2.service

install -D -m 644 "$ROOTFS_FILES/etc/systemd/system/getty@tty1.service.d/autologin.conf" \
    "$MNT_ROOTFS/etc/systemd/system/getty@tty1.service.d/autologin.conf"

run_in_chroot passwd -d root

install -D -m 644 "$ROOTFS_FILES/etc/systemd/logind.conf.d/reserve-tty2.conf" \
    "$MNT_ROOTFS/etc/systemd/logind.conf.d/reserve-tty2.conf"

echo "bes-installer" > "$MNT_ROOTFS/etc/hostname"
run_in_chroot systemd-machine-id-setup 2>/dev/null || true

# ============================================================
# Phase 4: Unmount chroot, clean up, produce tarball
# ============================================================
echo "==> Phase 4: Unmounting chroot and creating tarball..."
umount "$MNT_ROOTFS/dev/pts"
umount "$MNT_ROOTFS/dev"
umount "$MNT_ROOTFS/proc"
umount "$MNT_ROOTFS/sys"
umount "$MNT_ROOTFS/run"
CHROOT_MOUNTS_ACTIVE=0

rm -f "$MNT_ROOTFS/etc/resolv.conf"
ln -sf /run/systemd/resolve/stub-resolv.conf "$MNT_ROOTFS/etc/resolv.conf"

# r[impl iso.cleanup.logs]
find "$MNT_ROOTFS/var/log" -type f -delete

# r[impl iso.cleanup.passwd-backups]
rm -f "$MNT_ROOTFS/etc/passwd-" "$MNT_ROOTFS/etc/shadow-" "$MNT_ROOTFS/etc/group-" \
      "$MNT_ROOTFS/etc/gshadow-" "$MNT_ROOTFS/etc/subuid-" "$MNT_ROOTFS/etc/subgid-"

# r[impl iso.cleanup.dhcp-leases]
rm -rf "$MNT_ROOTFS/var/lib/dhcp/"*

rm -rf "$MNT_ROOTFS/tmp/"*
rm -rf "$MNT_ROOTFS/var/cache/apt/archives/"*.deb
rm -rf "$MNT_ROOTFS/var/lib/apt/lists/"*

mkdir -p "$(dirname "$OUTPUT")"
echo "    Creating tarball (this may take a while)..."
tar cf "$OUTPUT" -C "$MNT_ROOTFS" .
echo "    tarball: $(du -h "$OUTPUT" | cut -f1)"

# Clean up working directory
rm -rf "$WORK_DIR"
WORK_DIR=""

trap - EXIT

echo ""
echo "=============================="
echo "Base rootfs built successfully"
echo "=============================="
echo "Output: $OUTPUT ($(du -h "$OUTPUT" | cut -f1))"
echo "=============================="
