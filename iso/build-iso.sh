#!/bin/bash
# r[impl iso.base]
# r[impl iso.minimal]
# r[impl iso.offline]
# r[impl iso.contents]
# r[impl iso.boot.uefi]
# r[impl iso.boot.autostart]
# r[impl iso.efi-writable]
# r[impl iso.per-arch]
# r[impl iso.usb]
#
# Build a live installer image as a GPT disk image with:
#   - Partition 1: FAT32 EFI System Partition (GRUB EFI + config file location)
#   - Partition 2: ext4 data partition (squashfs live rootfs + compressed disk images + installer)
#
# The EFI partition is a real FAT32 partition, so after dd to USB it remains
# writable — users can place bes-install.toml on it before booting.
#
# Usage: build-iso.sh
#   Environment variables:
#     ARCH            - amd64 or arm64 (default: amd64)
#     OUTPUT          - output file path (default: output/<arch>/bes-installer-<arch>.img)
#     INSTALLER_BIN   - path to the bes-installer binary
#     IMAGE_DIR       - directory containing .raw.zst images to embed
#     UBUNTU_SUITE    - Ubuntu suite name (default: noble)
#     ESP_SIZE_MB     - EFI partition size in MiB (default: 64)
set -euo pipefail

ARCH="${ARCH:-amd64}"
UBUNTU_SUITE="${UBUNTU_SUITE:-noble}"
ESP_SIZE_MB="${ESP_SIZE_MB:-64}"
INSTALLER_BIN="${INSTALLER_BIN:?INSTALLER_BIN must point to the bes-installer binary}"
IMAGE_DIR="${IMAGE_DIR:?IMAGE_DIR must point to directory with .raw.zst images}"
OUTPUT="${OUTPUT:-output/${ARCH}/bes-installer-${ARCH}.iso}"

case "$ARCH" in
    amd64)
        UBUNTU_MIRROR="${UBUNTU_MIRROR:-http://archive.ubuntu.com/ubuntu}"
        GRUB_TARGET="x86_64-efi"
        GRUB_EFI_NAME="BOOTX64.EFI"
        ;;
    arm64)
        UBUNTU_MIRROR="${UBUNTU_MIRROR:-http://ports.ubuntu.com/ubuntu-ports}"
        GRUB_TARGET="arm64-efi"
        GRUB_EFI_NAME="BOOTAA64.EFI"
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

if [ ! -f "$INSTALLER_BIN" ]; then
    echo "ERROR: installer binary not found: $INSTALLER_BIN"
    exit 1
fi

if [ ! -d "$IMAGE_DIR" ]; then
    echo "ERROR: image directory not found: $IMAGE_DIR"
    exit 1
fi

IMAGE_FILES=()
while IFS= read -r -d '' f; do
    IMAGE_FILES+=("$f")
done < <(find "$IMAGE_DIR" -maxdepth 1 -name "*.raw.zst" -print0)

if [ "${#IMAGE_FILES[@]}" -eq 0 ]; then
    echo "ERROR: no .raw.zst images found in $IMAGE_DIR"
    exit 1
fi

MISSING=()
for cmd in debootstrap mksquashfs unsquashfs sgdisk mkfs.vfat mkfs.ext4 losetup grub-mkimage partprobe; do
    command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
done
if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "ERROR: missing required commands: ${MISSING[*]}"
    exit 1
fi

echo "=============================="
echo "BES Live ISO Builder"
echo "=============================="
echo "Architecture:  $ARCH"
echo "Output:        $OUTPUT"
echo "Installer:     $INSTALLER_BIN"
echo "Image dir:     $IMAGE_DIR"
echo "Images:        ${IMAGE_FILES[*]}"
echo "Suite:         $UBUNTU_SUITE"
echo "ESP size:      ${ESP_SIZE_MB} MiB"
echo "=============================="
echo ""

# ============================================================
# State tracking for cleanup
# ============================================================
WORK_DIR=""
LOOP_DEVICE=""
MNT_ROOTFS=""
MNT_ESP=""
MNT_DATA=""
CHROOT_MOUNTS_ACTIVE=0

cleanup() {
    local exit_code=$?
    echo ""
    if [ $exit_code -ne 0 ]; then
        echo "!!! Build failed (exit code $exit_code), cleaning up..."
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

    [ -n "$MNT_ESP" ] && mountpoint -q "$MNT_ESP" 2>/dev/null && umount "$MNT_ESP"
    [ -n "$MNT_DATA" ] && mountpoint -q "$MNT_DATA" 2>/dev/null && umount "$MNT_DATA"
    [ -n "$MNT_ROOTFS" ] && mountpoint -q "$MNT_ROOTFS" 2>/dev/null && umount "$MNT_ROOTFS"

    if [ -n "$LOOP_DEVICE" ]; then
        losetup -d "$LOOP_DEVICE" 2>/dev/null
    fi

    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi

    if [ $exit_code -ne 0 ]; then
        rm -f "$OUTPUT"
    fi
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-iso-XXXXXX)"
MNT_ROOTFS="$WORK_DIR/rootfs"
MNT_ESP="$WORK_DIR/esp"
MNT_DATA="$WORK_DIR/data"
STAGING="$WORK_DIR/staging"

mkdir -p "$MNT_ROOTFS" "$MNT_ESP" "$MNT_DATA" "$STAGING"

# ============================================================
# Phase 1: Build minimal live rootfs via debootstrap
# ============================================================
echo "==> Building minimal live rootfs..."

DEBOOTSTRAP_EXTRA_ARGS=()
if [ ! -f /usr/share/keyrings/ubuntu-archive-keyring.gpg ]; then
    echo "    (Ubuntu keyring not found on host — using --no-check-gpg)"
    DEBOOTSTRAP_EXTRA_ARGS+=(--no-check-gpg)
fi

debootstrap \
    --arch="$ARCH" \
    --variant=minbase \
    --include=ca-certificates \
    "${DEBOOTSTRAP_EXTRA_ARGS[@]}" \
    "$UBUNTU_SUITE" "$MNT_ROOTFS" "$UBUNTU_MIRROR"

# ============================================================
# Phase 2: Install packages in chroot
# ============================================================
echo "==> Installing live environment packages..."

mount -t proc proc "$MNT_ROOTFS/proc"
mount -t sysfs sysfs "$MNT_ROOTFS/sys"
mount --bind /dev "$MNT_ROOTFS/dev"
mount --bind /dev/pts "$MNT_ROOTFS/dev/pts"
mount -t tmpfs tmpfs "$MNT_ROOTFS/run"
CHROOT_MOUNTS_ACTIVE=1

if [ -f /etc/resolv.conf ]; then
    cp --dereference /etc/resolv.conf "$MNT_ROOTFS/etc/resolv.conf"
elif [ -f /run/systemd/resolve/stub-resolv.conf ]; then
    cp --dereference /run/systemd/resolve/stub-resolv.conf "$MNT_ROOTFS/etc/resolv.conf"
else
    echo "nameserver 1.1.1.1" > "$MNT_ROOTFS/etc/resolv.conf"
fi

# r[impl iso.minimal]
chroot "$MNT_ROOTFS" bash -c "
    export DEBIAN_FRONTEND=noninteractive
    export PATH='/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin'

    apt-get update -q

    apt-get install -y -q --no-install-recommends \
        linux-generic \
        initramfs-tools \
        systemd \
        systemd-sysv \
        dbus \
        udev \
        util-linux \
        parted \
        gdisk \
        zstd \
        cryptsetup \
        btrfs-progs \
        lvm2 \
        dosfstools \
        e2fsprogs \
        pciutils \
        usbutils \
        less

    apt-get clean
    rm -rf /var/lib/apt/lists/*

    echo '--- DEBUG: /boot contents after package install ---'
    ls -la /boot/ || true
    echo '--- DEBUG: kernel modules ---'
    ls /lib/modules/ || true
    echo '--- DEBUG: end ---'
"

# ============================================================
# Phase 3: Install the TUI installer and configure autostart
# ============================================================
echo "==> Installing TUI installer binary..."
install -m 755 "$INSTALLER_BIN" "$MNT_ROOTFS/usr/local/bin/bes-installer"

# r[impl iso.boot.autostart]
echo "==> Configuring auto-launch of installer on boot..."

# Create a systemd service that launches the installer on the main console
cat > "$MNT_ROOTFS/etc/systemd/system/bes-installer.service" << 'UNIT'
[Unit]
Description=BES Installer TUI
After=multi-user.target
ConditionPathExists=/usr/local/bin/bes-installer

[Service]
Type=simple
ExecStart=/usr/local/bin/bes-installer
StandardInput=tty
StandardOutput=tty
StandardError=tty
TTYPath=/dev/tty1
TTYReset=yes
TTYVHangup=yes
Restart=on-failure
RestartSec=2

[Install]
WantedBy=multi-user.target
UNIT

chroot "$MNT_ROOTFS" systemctl enable bes-installer.service

# Disable getty on tty1 so it doesn't compete with the installer
chroot "$MNT_ROOTFS" systemctl mask getty@tty1.service

# Set hostname for the live environment
echo "bes-installer" > "$MNT_ROOTFS/etc/hostname"

# Ensure we have a machine-id (live systems need one)
chroot "$MNT_ROOTFS" systemd-machine-id-setup 2>/dev/null || true

# ============================================================
# Phase 4: Unmount chroot and create squashfs
# ============================================================
echo "==> Unmounting chroot virtual filesystems..."
umount "$MNT_ROOTFS/dev/pts"
umount "$MNT_ROOTFS/dev"
umount "$MNT_ROOTFS/proc"
umount "$MNT_ROOTFS/sys"
umount "$MNT_ROOTFS/run"
CHROOT_MOUNTS_ACTIVE=0

rm -f "$MNT_ROOTFS/etc/resolv.conf"
echo "nameserver 1.1.1.1" > "$MNT_ROOTFS/etc/resolv.conf"

# Clean up build artifacts
rm -rf "$MNT_ROOTFS/tmp/"*
rm -rf "$MNT_ROOTFS/var/cache/apt/archives/"*.deb
rm -rf "$MNT_ROOTFS/var/lib/apt/lists/"*

echo "==> DEBUG: rootfs /boot contents before squashfs..."
ls -la "$MNT_ROOTFS/boot/" || true
echo "==> DEBUG: looking for vmlinuz..."
find "$MNT_ROOTFS/boot" -maxdepth 1 -name 'vmlinuz*' -ls 2>/dev/null || true
echo "==> DEBUG: looking for initrd..."
find "$MNT_ROOTFS/boot" -maxdepth 1 -name 'initrd*' -ls 2>/dev/null || true

# Copy vmlinuz and initrd out of rootfs BEFORE squashing (avoids unsquashfs issues)
echo "==> Copying kernel and initrd from rootfs..."
mkdir -p "$STAGING/live"

VMLINUZ="$(find "$MNT_ROOTFS/boot" -maxdepth 1 -name 'vmlinuz-*' -not -name '*.old' -type f | sort -V | tail -1)"
INITRD="$(find "$MNT_ROOTFS/boot" -maxdepth 1 -name 'initrd.img-*' -not -name '*.old' -type f | sort -V | tail -1)"

echo "    vmlinuz candidate: ${VMLINUZ:-NONE}"
echo "    initrd candidate:  ${INITRD:-NONE}"

if [ -z "$VMLINUZ" ] || [ -z "$INITRD" ]; then
    echo "ERROR: could not find vmlinuz or initrd in rootfs /boot"
    echo "Full /boot listing:"
    find "$MNT_ROOTFS/boot" -ls 2>/dev/null || true
    exit 1
fi

cp "$VMLINUZ" "$STAGING/live/vmlinuz"
cp "$INITRD" "$STAGING/live/initrd.img"
echo "    vmlinuz: $(du -h "$STAGING/live/vmlinuz" | cut -f1)"
echo "    initrd:  $(du -h "$STAGING/live/initrd.img" | cut -f1)"

echo "==> Creating squashfs..."
SQUASHFS="$STAGING/live/filesystem.squashfs"
mkdir -p "$STAGING/images"
mksquashfs "$MNT_ROOTFS" "$SQUASHFS" -comp xz -no-exports -noappend -quiet
rm -rf "$MNT_ROOTFS"
echo "    squashfs: $(du -h "$SQUASHFS" | cut -f1)"

# ============================================================
# Phase 5: Copy images into staging
# ============================================================
# r[impl iso.contents]
echo "==> Copying disk images into staging..."
for img in "${IMAGE_FILES[@]}"; do
    echo "    $(basename "$img")"
    cp "$img" "$STAGING/images/"
done

# vmlinuz and initrd were already copied before squashfs creation

# ============================================================
# Phase 6: Calculate image size and create disk image
# ============================================================
echo "==> Calculating image size..."
DATA_SIZE_KB=$(du -sk "$STAGING" | cut -f1)
# Add 20% headroom for filesystem overhead
DATA_SIZE_MB=$(( (DATA_SIZE_KB / 1024) * 120 / 100 + 64 ))
TOTAL_SIZE_MB=$(( ESP_SIZE_MB + DATA_SIZE_MB + 2 ))

echo "    Data partition: ~${DATA_SIZE_MB} MiB"
echo "    Total image:    ~${TOTAL_SIZE_MB} MiB"

mkdir -p "$(dirname "$OUTPUT")"
truncate -s "${TOTAL_SIZE_MB}M" "$OUTPUT"

# ============================================================
# Phase 7: Partition the image
# ============================================================
echo "==> Partitioning image (GPT)..."
LOOP_DEVICE="$(losetup -f --show -P "$OUTPUT")"
echo "    Loop device: $LOOP_DEVICE"

sgdisk --zap-all "$LOOP_DEVICE" >/dev/null

# EFI System Partition
sgdisk -n "1:0:+${ESP_SIZE_MB}M" \
    -t 1:C12A7328-F81F-11D2-BA4B-00A0C93EC93B \
    -c 1:EFI \
    "$LOOP_DEVICE" >/dev/null

# Data partition (Linux filesystem)
sgdisk -n 2:0:0 \
    -t 2:0FC63DAF-8483-4772-8E79-3D69D8477DE4 \
    -c 2:BESDATA \
    "$LOOP_DEVICE" >/dev/null

partprobe "$LOOP_DEVICE"
udevadm settle
sleep 1

ESP_PART="${LOOP_DEVICE}p1"
DATA_PART="${LOOP_DEVICE}p2"

# ============================================================
# Phase 8: Format and populate EFI partition
# ============================================================
# r[impl iso.efi-writable]
echo "==> Formatting EFI partition (FAT32)..."
mkfs.vfat -F 32 -n EFI "$ESP_PART" >/dev/null

mount "$ESP_PART" "$MNT_ESP"

# r[impl iso.boot.uefi]
echo "==> Installing GRUB EFI bootloader..."
mkdir -p "$MNT_ESP/EFI/BOOT"
mkdir -p "$MNT_ESP/boot/grub"

# Build a standalone GRUB EFI image
grub-mkimage \
    -o "$MNT_ESP/EFI/BOOT/$GRUB_EFI_NAME" \
    -O "$GRUB_TARGET" \
    -p /boot/grub \
    part_gpt fat ext2 normal boot linux configfile loopback search \
    search_label search_fs_uuid ls cat echo test true

cat > "$MNT_ESP/boot/grub/grub.cfg" << 'GRUBCFG'
set timeout=3
set default=0

menuentry "BES Installer" {
    search --label --no-floppy --set=datapart BESDATA
    linux ($datapart)/live/vmlinuz boot=live toram quiet
    initrd ($datapart)/live/initrd.img
}

menuentry "BES Installer (verbose)" {
    search --label --no-floppy --set=datapart BESDATA
    linux ($datapart)/live/vmlinuz boot=live toram
    initrd ($datapart)/live/initrd.img
}
GRUBCFG

umount "$MNT_ESP"

# ============================================================
# Phase 9: Format and populate data partition
# ============================================================
echo "==> Formatting data partition (ext4)..."
mkfs.ext4 -q -L BESDATA "$DATA_PART"

mount "$DATA_PART" "$MNT_DATA"

echo "==> Copying live filesystem and images to data partition..."
cp -a "$STAGING"/* "$MNT_DATA/"

sync
umount "$MNT_DATA"

# ============================================================
# Phase 10: Cleanup and finalize
# ============================================================
echo "==> Detaching loop device..."
losetup -d "$LOOP_DEVICE"
LOOP_DEVICE=""

rm -rf "$WORK_DIR"
WORK_DIR=""

trap - EXIT

echo ""
echo "=============================="
echo "Live ISO built successfully"
echo "=============================="
echo "Output: $OUTPUT"
echo "Size:   $(du -h "$OUTPUT" | cut -f1)"
echo "SHA256: $(sha256sum "$OUTPUT" | cut -d' ' -f1)"
echo ""
echo "Write to USB:"
echo "  sudo dd if=$OUTPUT of=/dev/sdX bs=4M status=progress"
echo ""
echo "To pre-configure, mount the first partition and place bes-install.toml:"
echo "  mount /dev/sdX1 /mnt && cp bes-install.toml /mnt/ && umount /mnt"
echo "=============================="
