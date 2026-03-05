#!/bin/bash
#
# Launch the interactive TUI installer inside a systemd-nspawn container
# with a loopback target disk, so it can be tried out directly in a
# terminal without spinning up a VM.
#
# Usage: try-installer-interactive.sh <iso> <arch> [disk-size] [installer-bin]
#   arch:          amd64 | arm64
#   disk-size:     size of the loopback target disk (default: 10G)
#   installer-bin: path to a locally-built bes-installer binary; when given,
#                  this replaces the binary baked into the ISO, so you can
#                  iterate on the installer without rebuilding the whole ISO.
#
# Requires: systemd-nspawn, xorriso, unsquashfs, losetup, lsblk.
#           Must run as root.
set -euo pipefail

ISO="${1:?Usage: $0 <iso> <arch> [disk-size] [installer-bin]}"
ARCH="${2:?Usage: $0 <iso> <arch> [disk-size] [installer-bin]}"
TARGET_DISK_SIZE="${3:-10G}"
INSTALLER_BIN="${4:-}"

if [ ! -f "$ISO" ]; then
    echo "ERROR: ISO not found: $ISO"
    exit 1
fi

case "$ARCH" in
    amd64|arm64) ;;
    *)
        echo "ERROR: arch must be amd64 or arm64 (got: $ARCH)"
        exit 1
        ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root"
    exit 1
fi

MISSING=()
for cmd in systemd-nspawn xorriso unsquashfs losetup lsblk; do
    command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
done
if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "ERROR: missing required commands: ${MISSING[*]}"
    exit 1
fi

# ============================================================
# State tracking for cleanup
# ============================================================
WORK_DIR=""
LOOP_DEV=""
DEV_OVERLAY=""

cleanup() {
    local exit_code=$?
    set +e

    if [ -n "$DEV_OVERLAY" ] && mountpoint -q "$DEV_OVERLAY" 2>/dev/null; then
        umount "$DEV_OVERLAY" 2>/dev/null
    fi

    if [ -n "$LOOP_DEV" ]; then
        losetup -d "$LOOP_DEV" 2>/dev/null
    fi

    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi

    exit "$exit_code"
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-try-installer-XXXXXX)"
mkdir -p "$WORK_DIR/log"

echo "=============================="
echo "BES Interactive Installer Trial"
echo "=============================="
echo "ISO:        $ISO"
echo "Arch:       $ARCH"
echo "Disk size:  $TARGET_DISK_SIZE"
if [ -n "$INSTALLER_BIN" ]; then
echo "Installer:  $INSTALLER_BIN (override)"
else
echo "Installer:  (from ISO)"
fi
echo "Work dir:   $WORK_DIR"
echo "=============================="
echo ""

# ============================================================
# Phase 1: Extract rootfs and images from ISO
# ============================================================
echo "==> Extracting rootfs and images from ISO..."

SQUASHFS="$WORK_DIR/filesystem.squashfs"
ROOTFS_DIR="$WORK_DIR/rootfs"
IMAGES_DIR="$WORK_DIR/images"

xorriso -osirrox on -indev "$ISO" \
    -extract /live/filesystem.squashfs "$SQUASHFS" \
    2>/dev/null

if [ ! -f "$SQUASHFS" ]; then
    echo "ERROR: failed to extract /live/filesystem.squashfs from ISO"
    exit 1
fi

echo "    Extracted squashfs: $(du -h "$SQUASHFS" | cut -f1)"

unsquashfs -d "$ROOTFS_DIR" -f "$SQUASHFS" >/dev/null 2>&1
rm -f "$SQUASHFS"
echo "    Unpacked rootfs to $ROOTFS_DIR"

mkdir -p "$IMAGES_DIR"
xorriso -osirrox on -indev "$ISO" \
    -extract /images "$IMAGES_DIR" \
    2>/dev/null

IMAGE_COUNT=$(find "$IMAGES_DIR" -name '*.raw.zst' | wc -l)
if [ "$IMAGE_COUNT" -eq 0 ]; then
    echo "ERROR: no .raw.zst images found in ISO /images/"
    exit 1
fi
echo "    Extracted $IMAGE_COUNT disk image(s)"
echo ""

# ============================================================
# Phase 2: Create loopback target disk
# ============================================================
echo "==> Creating loopback target disk ($TARGET_DISK_SIZE)..."

TARGET_IMG="$WORK_DIR/target.img"
truncate -s "$TARGET_DISK_SIZE" "$TARGET_IMG"

LOOP_DEV="$(losetup --show --find --partscan "$TARGET_IMG")"
echo "    Loop device: $LOOP_DEV"

LOOP_SIZE_BYTES=$(blockdev --getsize64 "$LOOP_DEV")

# ============================================================
# Phase 3: Generate fake devices JSON
# ============================================================
echo "==> Generating fake devices list..."

DEVICES_JSON="$WORK_DIR/devices.json"
cat > "$DEVICES_JSON" << EOF
[
  {
    "path": "$LOOP_DEV",
    "size_bytes": $LOOP_SIZE_BYTES,
    "model": "Test Loopback Disk",
    "transport": "virtio"
  }
]
EOF

echo "    $DEVICES_JSON"
echo ""

# ============================================================
# Phase 4: Prepare rootfs
# ============================================================
echo "==> Preparing rootfs..."

if [ ! -f "$ROOTFS_DIR/etc/os-release" ] && [ ! -f "$ROOTFS_DIR/usr/lib/os-release" ]; then
    echo "BES Installer Live" > "$ROOTFS_DIR/etc/os-release"
fi

# Install a reboot trap so the container does not actually reboot the host.
for reboot_path in /usr/local/sbin/reboot /usr/sbin/reboot /sbin/reboot; do
    mkdir -p "$ROOTFS_DIR/$(dirname "$reboot_path")"
    cat > "$ROOTFS_DIR/$reboot_path" << 'TRAP'
#!/bin/sh
echo ""
echo "========================================="
echo "  Installation complete (reboot caught)"
echo "========================================="
echo ""
TRAP
    chmod +x "$ROOTFS_DIR/$reboot_path"
done

# If a local installer binary was provided, copy it into the rootfs,
# replacing the one that was baked into the ISO.
if [ -n "$INSTALLER_BIN" ]; then
    cp "$INSTALLER_BIN" "$ROOTFS_DIR/usr/local/bin/bes-installer"
    chmod +x "$ROOTFS_DIR/usr/local/bin/bes-installer"
    echo "    Replaced installer binary with $INSTALLER_BIN"
fi

echo "    Rootfs ready"
echo ""

# ============================================================
# Phase 5: Build /dev overlay that masks host block devices
# ============================================================
# The installer needs access to dynamically-created device nodes (partition
# sub-devices from partprobe, device-mapper nodes from cryptsetup), which
# requires a live view of devtmpfs. But we must not expose host disks.
#
# Solution: mount an overlayfs with the host /dev as the lower layer and
# delete (whiteout) all host block devices and device-mapper entries from the
# merged view. New nodes created by the kernel (loop partitions, dm-* from
# cryptsetup) appear through the lower layer automatically.
echo "==> Building masked /dev overlay..."

DEV_OVERLAY="$WORK_DIR/dev-overlay/merged"
mkdir -p "$WORK_DIR/dev-overlay/upper" "$WORK_DIR/dev-overlay/work" "$DEV_OVERLAY"

mount -t overlay overlay \
    -o "lowerdir=/dev,upperdir=$WORK_DIR/dev-overlay/upper,workdir=$WORK_DIR/dev-overlay/work" \
    "$DEV_OVERLAY"

# Mask host disk block devices (nvme, scsi, virtio, ide, mmc, existing dm-*)
MASKED=0
for dev in "$DEV_OVERLAY"/nvme* "$DEV_OVERLAY"/sd* "$DEV_OVERLAY"/vd* \
           "$DEV_OVERLAY"/hd* "$DEV_OVERLAY"/mmcblk* "$DEV_OVERLAY"/dm-*; do
    [ -e "$dev" ] || [ -L "$dev" ] || continue
    rm -f "$dev" 2>/dev/null && MASKED=$((MASKED + 1))
done

# Mask existing device-mapper symlinks (keep /dev/mapper/control for cryptsetup)
for entry in "$DEV_OVERLAY"/mapper/*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    name=$(basename "$entry")
    [ "$name" = "control" ] && continue
    rm -f "$entry" 2>/dev/null && MASKED=$((MASKED + 1))
done

# Clean up dangling symlinks left by systemd (gpt-auto-root etc.)
find "$DEV_OVERLAY" -maxdepth 1 -xtype l -delete 2>/dev/null || true

echo "    Masked $MASKED host device(s)"
echo "    Overlay at $DEV_OVERLAY"
echo ""

# ============================================================
# Phase 6: Launch interactive installer
# ============================================================
echo "==> Launching interactive installer in container..."
echo "    (The installer TUI will take over the terminal.)"
echo ""

# nspawn options: --pipe instead of --console=interactive so that the
# bind-mounted /dev overlay does not conflict with nspawn's /dev/console
# setup. The TUI works fine in pipe mode — crossterm operates on the
# inherited terminal fd directly. --private-network is omitted so
# tailscale netcheck works.
NSPAWN_OPTS=(
    --register=no
    --quiet
    --pipe
    --capability=CAP_SYS_ADMIN
    --system-call-filter=mount
    --property=DeviceAllow='block-loop rwm'
    --property=DeviceAllow='block-blkext rwm'
    --property=DeviceAllow='char-misc rwm'
    --property=DeviceAllow='block-device-mapper rwm'
)

NSPAWN_BINDS=(
    "--bind=$LOOP_DEV"
    "--bind=$DEV_OVERLAY:/dev"
    "--bind=$WORK_DIR/log:/log"
    "--bind-ro=$IMAGES_DIR:/run/live/medium/images"
    "--bind-ro=$DEVICES_JSON:/tmp/devices.json"
)

set -x
set +e
systemd-nspawn \
    "${NSPAWN_OPTS[@]}" \
    --directory="$ROOTFS_DIR" \
    "${NSPAWN_BINDS[@]}" \
    /usr/local/bin/bes-installer \
        --fake-devices /tmp/devices.json \
        --fake-tpm \
        --no-reboot \
        --log /log/installer.log
RC=$?
set -e
set +x

echo ""
if [ -f "$WORK_DIR/log/installer.log" ]; then
    echo "Installer log saved to: $WORK_DIR/log/installer.log"
    echo ""
    echo "=== Installer log ==="
    cat  "$WORK_DIR/log/installer.log"
    echo "=== End installer log ==="
    echo ""
fi

if [ $RC -eq 0 ]; then
    echo "Installer exited successfully."
else
    echo "Installer exited with code $RC."
fi
