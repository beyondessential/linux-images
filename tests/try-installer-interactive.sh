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
# Requires: systemd-nspawn, xorriso, unsquashfs, losetup, lsblk, sgdisk,
#           veritysetup. Must run as root.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/nspawn-opts.sh"
source "$SCRIPT_DIR/iso-images-mount.sh"

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
for cmd in systemd-nspawn xorriso unsquashfs losetup lsblk sgdisk veritysetup; do
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

cleanup() {
    local exit_code=$?
    set +e

    iso_images_cleanup

    # Unmount any stale mounts the installer left on the target device or
    # its LUKS mapper before closing LUKS / detaching the loop.
    if [ -n "$LOOP_DEV" ]; then
        grep "$LOOP_DEV\|bes-target-root" /proc/mounts 2>/dev/null \
            | awk '{print $2}' | sort -r | while read -r mp; do
            umount "$mp" 2>/dev/null
        done
    fi

    if [ -e /dev/mapper/bes-target-root ]; then
        cryptsetup close bes-target-root 2>/dev/null
    fi

    if [ -n "$LOOP_DEV" ]; then
        losetup -d "$LOOP_DEV" 2>/dev/null
    fi

    swtpm_stop

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

# Mount the images squashfs from the ISO's GPT images partition via dm-verity.
# This verifies integrity and gives us the real squashfs mount that the
# installer would see in production at /run/bes-images.
iso_images_mount "$ISO"
IMAGES_DIR="$ISO_IMAGES_MNT"
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
# Phase 5: Start software TPM if available
# ============================================================
# Try to start swtpm so that TPM encryption works end-to-end in the
# interactive trial. If swtpm or the tpm_vtpm_proxy kernel module is
# not available, fall back to --fake-tpm (the TUI will still default to
# TPM encryption, but enrollment will fail at the end).
INSTALLER_TPM_ARGS=(--fake-tpm)
if swtpm_start "$WORK_DIR/swtpm" 2>/dev/null; then
    echo "==> Software TPM started — TPM encryption will work end-to-end."
else
    echo "==> Software TPM not available (missing swtpm or tpm_vtpm_proxy module)."
    echo "    TPM enrollment will fail if selected."
fi
echo ""

# ============================================================
# Phase 6: Launch interactive installer
# ============================================================
echo "==> Launching interactive installer in container..."
echo "    (The installer TUI will take over the terminal.)"
echo ""

# r[impl installer.container.isolation+4]: use the shared nspawn
# configuration. --private-network is omitted so tailscale netcheck works.
#
# The container gets nspawn's own private /dev (no host devices exposed).
# After partprobe, partition device nodes only appear on the host's devtmpfs,
# not inside the container. The installer handles this by reading
# /sys/class/block/ and creating missing device nodes via mknod
# (see r[installer.container.partition-devices+3]).
nspawn_opts
nspawn_installer_binds "$LOOP_DEV" "$IMAGES_DIR" "$DEVICES_JSON" \
    "" "$WORK_DIR/log:/log"

set +e
systemd-nspawn \
    "${NSPAWN_OPTS[@]}" \
    --directory="$ROOTFS_DIR" \
    "${NSPAWN_BINDS[@]}" \
    /usr/local/bin/bes-installer \
        --fake-devices /tmp/devices.json \
        "${INSTALLER_TPM_ARGS[@]}" \
        --no-reboot \
        --log /log/installer.log
RC=$?
set -e

echo ""
if [ -f "$WORK_DIR/log/installer.log" ]; then
    INSTALLER_LOG="$WORK_DIR/log/installer.log"
    echo "Installer log saved to: $INSTALLER_LOG"
    echo ""
    echo "=== Installer log (last 40 lines) ==="
    tail -40 "$INSTALLER_LOG"
    echo "=== End installer log ==="
    echo ""
fi

if [ $RC -eq 0 ]; then
    echo "Installer exited successfully."
else
    echo "Installer exited with code $RC."
fi
