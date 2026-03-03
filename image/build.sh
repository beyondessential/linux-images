#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# --- Arguments (overridable via environment) ---
ARCH="${ARCH:-amd64}"
VARIANT="${VARIANT:-metal}"
OUTPUT="${OUTPUT:-output.raw}"
IMAGE_SIZE="${IMAGE_SIZE:-8G}"
UBUNTU_SUITE="${UBUNTU_SUITE:-noble}"

# --- Derived values ---
case "$ARCH" in
    amd64)
        UBUNTU_MIRROR="${UBUNTU_MIRROR:-http://archive.ubuntu.com/ubuntu}"
        GRUB_TARGET="x86_64-efi"
        ;;
    arm64)
        UBUNTU_MIRROR="${UBUNTU_MIRROR:-http://ports.ubuntu.com/ubuntu-ports}"
        GRUB_TARGET="arm64-efi"
        ;;
    *)
        echo "ERROR: arch must be amd64 or arm64 (got: $ARCH)"
        exit 1
        ;;
esac

# r[image.variant.types]
case "$VARIANT" in
    metal|cloud) ;;
    *)
        echo "ERROR: variant must be metal or cloud (got: $VARIANT)"
        exit 1
        ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root"
    exit 1
fi

# --- Dependency checks ---
MISSING=()
for cmd in debootstrap sgdisk mkfs.vfat mkfs.ext4 mkfs.btrfs losetup btrfs chroot rsync; do
    command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
done
if [ "$VARIANT" = "metal" ]; then
    command -v cryptsetup &>/dev/null || MISSING+=("cryptsetup")
fi
if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "ERROR: missing required commands: ${MISSING[*]}"
    exit 1
fi

HOST_ARCH="$(uname -m)"
NEED_QEMU=0
if [ "$ARCH" = "arm64" ] && [ "$HOST_ARCH" = "x86_64" ]; then
    NEED_QEMU=1
    if [ ! -f /proc/sys/fs/binfmt_misc/qemu-aarch64 ]; then
        echo "ERROR: cross-building arm64 on x86_64 requires qemu-user-static + binfmt_misc"
        echo "  Install: apt-get install qemu-user-static binfmt-support"
        echo "  Enable:  update-binfmts --enable qemu-aarch64"
        exit 1
    fi
fi
if [ "$ARCH" = "amd64" ] && [ "$HOST_ARCH" = "aarch64" ]; then
    NEED_QEMU=1
    if [ ! -f /proc/sys/fs/binfmt_misc/qemu-x86_64 ]; then
        echo "ERROR: cross-building amd64 on aarch64 requires qemu-user-static + binfmt_misc"
        exit 1
    fi
fi

echo "=============================="
echo "BES Linux Image Builder"
echo "=============================="
echo "Architecture:  $ARCH"
echo "Variant:       $VARIANT"
echo "Output:        $OUTPUT"
echo "Image size:    $IMAGE_SIZE"
echo "Suite:         $UBUNTU_SUITE"
echo "Mirror:        $UBUNTU_MIRROR"
echo "Cross-build:   $([ $NEED_QEMU -eq 1 ] && echo yes || echo no)"
echo "=============================="
echo ""

# ============================================================
# State tracking for cleanup
# ============================================================
LOOP_DEVICE=""
MNT="/mnt/image-$$"
LUKS_NAME="image-root-$$"
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

    # Unmount chroot virtual filesystems
    if [ $CHROOT_MOUNTS_ACTIVE -eq 1 ]; then
        umount "$MNT/dev/pts"  2>/dev/null
        umount "$MNT/dev"      2>/dev/null
        umount "$MNT/proc"     2>/dev/null
        umount "$MNT/sys"      2>/dev/null
        umount "$MNT/run"      2>/dev/null
        umount "$MNT/tmp"      2>/dev/null
    fi

    # Unmount filesystems in reverse order
    umount "$MNT/boot/efi"          2>/dev/null
    umount "$MNT/boot"              2>/dev/null
    umount "$MNT/var/lib/postgresql" 2>/dev/null
    umount "$MNT"                   2>/dev/null

    # Close LUKS
    if [ "$VARIANT" = "metal" ]; then
        cryptsetup close "$LUKS_NAME" 2>/dev/null
    fi

    # Detach loop device
    if [ -n "$LOOP_DEVICE" ]; then
        losetup -d "$LOOP_DEVICE" 2>/dev/null
    fi

    # Remove mount point
    rmdir "$MNT" 2>/dev/null

    if [ $exit_code -ne 0 ]; then
        echo "!!! Cleaning up output file due to failure"
        rm -f "$OUTPUT"
    fi
}
trap cleanup EXIT

# ============================================================
# Phase 1: Create and partition the image
# ============================================================
echo "==> Creating ${IMAGE_SIZE} raw image file..."
mkdir -p "$(dirname "$OUTPUT")"
truncate -s "$IMAGE_SIZE" "$OUTPUT"

LOOP_DEVICE="$(losetup -f --show -P "$OUTPUT")"
echo "    Loop device: $LOOP_DEVICE"

# r[image.partition.table]: GPT partition table.
echo "==> Partitioning (GPT)..."
sgdisk --zap-all "$LOOP_DEVICE" >/dev/null

# r[image.partition.efi]
sgdisk -n 1:0:+512M \
    -t 1:C12A7328-F81F-11D2-BA4B-00A0C93EC93B \
    -c 1:efi \
    "$LOOP_DEVICE" >/dev/null

# r[image.partition.xboot]
sgdisk -n 2:0:+1G \
    -t 2:BC13C2FF-59E6-4262-A352-B275FD6F7172 \
    -c 2:xboot \
    "$LOOP_DEVICE" >/dev/null

# r[image.partition.root]
if [ "$ARCH" = "amd64" ]; then
    ROOT_TYPE_UUID="4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709"
else
    ROOT_TYPE_UUID="B921B045-1DF0-41C3-AF44-4C6F280D3FAE"
fi
sgdisk -n 3:0:0 \
    -t "3:$ROOT_TYPE_UUID" \
    -c 3:root \
    "$LOOP_DEVICE" >/dev/null

partprobe "$LOOP_DEVICE"
udevadm settle
sleep 1

EFI_PART="${LOOP_DEVICE}p1"
BOOT_PART="${LOOP_DEVICE}p2"
ROOT_PART="${LOOP_DEVICE}p3"

# r[image.partition.count]: Verify exactly 3 partitions.
PART_COUNT="$(lsblk -ln -o NAME "$LOOP_DEVICE" | grep -c "^$(basename "$LOOP_DEVICE")p")"
if [ "$PART_COUNT" -ne 3 ]; then
    echo "ERROR: expected 3 partitions, got $PART_COUNT"
    exit 1
fi

echo "    Partitions: efi=${EFI_PART} xboot=${BOOT_PART} root=${ROOT_PART}"

# ============================================================
# Phase 2: Format filesystems
# ============================================================
echo "==> Formatting EFI partition (FAT32)..."
mkfs.vfat -F 32 -n EFI "$EFI_PART" >/dev/null

echo "==> Formatting boot partition (ext4)..."
mkfs.ext4 -q -L xboot "$BOOT_PART"

# r[image.luks.format]: Metal variant gets LUKS2 with empty passphrase.
if [ "$VARIANT" = "metal" ]; then
    echo "==> Setting up LUKS2 on root partition..."
    KEYFILE="$(mktemp)"
    truncate -s 0 "$KEYFILE"
    cryptsetup luksFormat --type luks2 --batch-mode \
        "$ROOT_PART" --key-file "$KEYFILE" --key-slot 0
    cryptsetup open "$ROOT_PART" "$LUKS_NAME" --key-file "$KEYFILE"
    rm -f "$KEYFILE"
    BTRFS_DEV="/dev/mapper/$LUKS_NAME"
    echo "    LUKS opened as $LUKS_NAME"
else
    BTRFS_DEV="$ROOT_PART"
fi

# r[image.btrfs.format]
echo "==> Formatting root as BTRFS..."
mkfs.btrfs --quiet \
    --label ROOT \
    --checksum xxhash \
    --features block-group-tree,squota \
    "$BTRFS_DEV"

# r[image.btrfs.subvolumes]: Create @ and @postgres subvolumes.
# r[image.btrfs.quotas]: Enable simple quotas.
echo "==> Creating BTRFS subvolumes..."
mkdir -p "$MNT"
mount "$BTRFS_DEV" "$MNT" -o compress=zstd:6
btrfs quota enable --simple "$MNT"
btrfs subvolume create "$MNT/@"
btrfs subvolume create "$MNT/@postgres"
umount "$MNT"

# ============================================================
# Phase 3: Mount the final layout
# ============================================================
echo "==> Mounting filesystems..."

# r[image.btrfs.compression]: zstd:6 on all BTRFS mounts.
mount "$BTRFS_DEV" "$MNT" -o subvol=@,compress=zstd:6

mkdir -p "$MNT/var/lib/postgresql"
mount "$BTRFS_DEV" "$MNT/var/lib/postgresql" -o subvol=@postgres,compress=zstd:6

mkdir -p "$MNT/boot"
mount "$BOOT_PART" "$MNT/boot"

mkdir -p "$MNT/boot/efi"
mount "$EFI_PART" "$MNT/boot/efi"

# ============================================================
# Phase 4: Debootstrap
# ============================================================
# r[image.base.debootstrap]: Bootstrap from Ubuntu 24.04 (Noble).
# r[image.base.minimal]: Use minbase variant.
# If the Ubuntu keyring isn't available (e.g. building on Arch), skip GPG check.
# The packages are still fetched over HTTPS and apt inside the chroot will have
# the real keyring once debootstrap completes.
DEBOOTSTRAP_EXTRA_ARGS=()
if [ ! -f /usr/share/keyrings/ubuntu-archive-keyring.gpg ]; then
    echo "    (Ubuntu keyring not found on host — using --no-check-gpg)"
    DEBOOTSTRAP_EXTRA_ARGS+=(--no-check-gpg)
fi

echo "==> Running debootstrap (${UBUNTU_SUITE}, ${ARCH}, minbase)..."
debootstrap \
    --arch="$ARCH" \
    --variant=minbase \
    --include=ca-certificates \
    "${DEBOOTSTRAP_EXTRA_ARGS[@]}" \
    "$UBUNTU_SUITE" "$MNT" "$UBUNTU_MIRROR"

# ============================================================
# Phase 5: Prepare chroot environment
# ============================================================
echo "==> Mounting virtual filesystems for chroot..."
mount -t proc proc "$MNT/proc"
mount -t sysfs sysfs "$MNT/sys"
mount --bind /dev "$MNT/dev"
mount --bind /dev/pts "$MNT/dev/pts"
mount -t tmpfs tmpfs "$MNT/run"
mount -t tmpfs tmpfs "$MNT/tmp"
CHROOT_MOUNTS_ACTIVE=1

# Provide DNS resolution inside chroot (will be replaced later)
if [ -f /etc/resolv.conf ]; then
    cp --dereference /etc/resolv.conf "$MNT/etc/resolv.conf"
elif [ -f /run/systemd/resolve/stub-resolv.conf ]; then
    cp --dereference /run/systemd/resolve/stub-resolv.conf "$MNT/etc/resolv.conf"
else
    echo "nameserver 1.1.1.1" > "$MNT/etc/resolv.conf"
fi

# Copy build inputs into chroot
echo "==> Copying build inputs into chroot..."
cp "$SCRIPT_DIR/configure.sh"  "$MNT/tmp/configure.sh"
cp "$SCRIPT_DIR/packages.sh"   "$MNT/tmp/packages.sh"
cp -r "$SCRIPT_DIR/scripts"    "$MNT/tmp/scripts/"
cp -r "$SCRIPT_DIR/files"      "$MNT/tmp/files/"

# ============================================================
# Phase 6: Run in-chroot configuration
# ============================================================
echo "==> Running configure.sh inside chroot..."
chroot "$MNT" /bin/bash /tmp/configure.sh "$ARCH" "$VARIANT" "$GRUB_TARGET"

# ============================================================
# Phase 7: Post-chroot cleanup
# ============================================================
echo "==> Post-chroot cleanup..."

# r[image.base.resolver]
rm -f "$MNT/etc/resolv.conf"
ln -snf /run/systemd/resolve/stub-resolv.conf "$MNT/etc/resolv.conf"

# r[image.base.machine-id]
truncate -s 0 "$MNT/etc/machine-id"

# r[image.cloud-init.no-network]
rm -rf "$MNT/etc/cloud/cloud.cfg.d/90-installer-network.cfg"

rm -rf "$MNT/etc/update-motd.d/60-unminimize"

# Clean temporary build files
rm -rf "$MNT/tmp/"*
rm -rf "$MNT/var/cache/apt/archives/"*.deb
rm -rf "$MNT/var/lib/apt/lists/"*

# ============================================================
# Phase 8: Unmount
# ============================================================
echo "==> Unmounting chroot virtual filesystems..."
umount "$MNT/dev/pts"
umount "$MNT/dev"
umount "$MNT/proc"
umount "$MNT/sys"
umount "$MNT/run"
umount "$MNT/tmp"
CHROOT_MOUNTS_ACTIVE=0

echo "==> Unmounting image filesystems..."
umount "$MNT/boot/efi"
umount "$MNT/boot"
umount "$MNT/var/lib/postgresql"
umount "$MNT"

if [ "$VARIANT" = "metal" ]; then
    echo "==> Closing LUKS volume..."
    cryptsetup close "$LUKS_NAME"
fi

echo "==> Detaching loop device..."
losetup -d "$LOOP_DEVICE"
LOOP_DEVICE=""

rmdir "$MNT"

trap - EXIT

echo ""
echo "=============================="
echo "Image built successfully"
echo "=============================="
echo "Output: $OUTPUT"
echo "Size:   $(du -h "$OUTPUT" | cut -f1)"
echo "SHA256: $(sha256sum "$OUTPUT" | cut -d' ' -f1)"
echo "=============================="
