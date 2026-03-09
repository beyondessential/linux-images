#!/bin/bash
#
# Build the live installer rootfs from a pre-built base tarball: unpack the
# base, inject the installer binary and build metadata, create a squashfs,
# and add dm-verity.
#
# This is the fast step (~30s) that runs every time the installer changes,
# while the slow base build (debootstrap + apt-get) is cached as a tarball.
#
# Output: a staging directory (OUTPUT_DIR) containing:
#   live/vmlinuz              - kernel
#   live/initrd.img           - initramfs (with verity hook)
#   live/filesystem.squashfs  - squashfs with appended verity hash tree + trailer
#   live/verity-roothash      - text file with the hex root hash
#
# Usage: build-iso-rootfs.sh
#   Environment variables:
#     ARCH          - amd64 or arm64 (default: amd64)
#     OUTPUT_DIR    - output staging directory (required)
#     BASE_TARBALL  - path to the base rootfs tarball from build-iso-base.sh (required)
#     INSTALLER_BIN - path to the bes-installer binary (required)
set -euo pipefail

ARCH="${ARCH:-amd64}"
BUILD_DATE="$(date -u +%Y-%m-%d)"
OUTPUT_DIR="${OUTPUT_DIR:?OUTPUT_DIR must be set to the rootfs staging directory}"
BASE_TARBALL="${BASE_TARBALL:?BASE_TARBALL must point to the base rootfs tarball}"
INSTALLER_BIN="${INSTALLER_BIN:?INSTALLER_BIN must point to the bes-installer binary}"

case "$ARCH" in
    amd64|arm64) ;;
    *)
        echo "ERROR: ARCH must be amd64 or arm64 (got: $ARCH)"
        exit 1
        ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root"
    exit 1
fi

if [ ! -f "$BASE_TARBALL" ]; then
    echo "ERROR: base tarball not found: $BASE_TARBALL"
    exit 1
fi

if [ ! -f "$INSTALLER_BIN" ]; then
    echo "ERROR: installer binary not found: $INSTALLER_BIN"
    exit 1
fi

MISSING=()
for cmd in mksquashfs veritysetup; do
    command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
done
if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "ERROR: missing required commands: ${MISSING[*]}"
    exit 1
fi

echo "=============================="
echo "BES Live ISO — Rootfs Builder"
echo "=============================="
echo "Architecture:  $ARCH"
echo "Output dir:    $OUTPUT_DIR"
echo "Base tarball:  $BASE_TARBALL ($(du -h "$BASE_TARBALL" | cut -f1))"
echo "Installer:     $INSTALLER_BIN"
echo "Build date:    $BUILD_DATE"
echo "=============================="
echo ""

# ============================================================
# State tracking for cleanup
# ============================================================
WORK_DIR=""

cleanup() {
    local exit_code=$?
    echo ""
    if [ $exit_code -ne 0 ]; then
        echo "!!! Rootfs build failed (exit code $exit_code), cleaning up..."
    else
        echo "Cleaning up..."
    fi

    set +e

    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi

    if [ $exit_code -ne 0 ]; then
        rm -rf "$OUTPUT_DIR"
    fi
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-rootfs-XXXXXX)"
ROOTFS="$WORK_DIR/rootfs"

mkdir -p "$ROOTFS" "$OUTPUT_DIR/live"

# ============================================================
# Phase 1: Unpack base tarball
# ============================================================
echo "==> Phase 1: Unpacking base tarball..."
tar xf "$BASE_TARBALL" -C "$ROOTFS"
echo "    rootfs: $(du -sh "$ROOTFS" | cut -f1)"

# ============================================================
# Phase 2: Inject installer binary and build metadata
# ============================================================
echo "==> Phase 2: Installing TUI installer binary..."
install -m 755 "$INSTALLER_BIN" "$ROOTFS/usr/local/bin/bes-installer"

cat > "$ROOTFS/etc/bes-build-info" << BUILDINFO
BUILD_DATE=$BUILD_DATE
ARCH=$ARCH
BUILDINFO

# ============================================================
# Phase 3: Extract kernel + initrd, create squashfs
# ============================================================
echo "==> Phase 3: Creating squashfs..."

VMLINUZ="$(find "$ROOTFS/boot" -maxdepth 1 -name 'vmlinuz-*' -not -name '*.old' -type f | sort -V | tail -1)"
INITRD="$(find "$ROOTFS/boot" -maxdepth 1 -name 'initrd.img-*' -not -name '*.old' -type f | sort -V | tail -1)"

if [ -z "$VMLINUZ" ] || [ -z "$INITRD" ]; then
    echo "ERROR: could not find vmlinuz or initrd in rootfs /boot"
    echo "Full /boot listing:"
    find "$ROOTFS/boot" -ls 2>/dev/null || true
    exit 1
fi

cp "$VMLINUZ" "$OUTPUT_DIR/live/vmlinuz"
cp "$INITRD" "$OUTPUT_DIR/live/initrd.img"
echo "    vmlinuz: $(du -h "$OUTPUT_DIR/live/vmlinuz" | cut -f1)"
echo "    initrd:  $(du -h "$OUTPUT_DIR/live/initrd.img" | cut -f1)"

echo "    Creating squashfs (this may take a while)..."
mksquashfs "$ROOTFS" "$OUTPUT_DIR/live/filesystem.squashfs" \
    -comp xz -no-exports -noappend -quiet
rm -rf "$ROOTFS"
echo "    squashfs: $(du -h "$OUTPUT_DIR/live/filesystem.squashfs" | cut -f1)"

# ============================================================
# Phase 4: Add verity to squashfs rootfs
# ============================================================
# r[impl iso.verity.squashfs+3]
# r[impl iso.verity.layout+3]
echo "==> Phase 4: Adding verity to squashfs rootfs..."

SQFS_HASHTREE="$WORK_DIR/filesystem.squashfs.hashtree"
SQFS_DATA_SIZE="$(stat --format='%s' "$OUTPUT_DIR/live/filesystem.squashfs")"
SQFS_VERITY_OUTPUT="$(veritysetup format "$OUTPUT_DIR/live/filesystem.squashfs" "$SQFS_HASHTREE" 2>&1)"
LIVE_ROOTHASH="$(echo "$SQFS_VERITY_OUTPUT" | grep "Root hash:" | awk '{print $NF}')"
echo "    live verity root hash: $LIVE_ROOTHASH"

# r[impl iso.verity.layout+3]
# Append hash tree + sector-aligned trailer. The blob must be padded to a
# 4096-byte boundary so that losetup does not truncate the trailing bytes
# and the verity trailer remains at exactly total_size - 8.
cat "$SQFS_HASHTREE" >> "$OUTPUT_DIR/live/filesystem.squashfs"
rm -f "$SQFS_HASHTREE"

SQFS_CURRENT_SIZE="$(stat --format='%s' "$OUTPUT_DIR/live/filesystem.squashfs")"
SQFS_TOTAL_NEEDED=$(python3 -c "
cur = $SQFS_CURRENT_SIZE + 8
aligned = ((cur + 4095) // 4096) * 4096
print(aligned)
")
SQFS_PADDING=$((SQFS_TOTAL_NEEDED - SQFS_CURRENT_SIZE - 8))
if [ "$SQFS_PADDING" -gt 0 ]; then
    dd if=/dev/zero bs=1 count="$SQFS_PADDING" 2>/dev/null >> "$OUTPUT_DIR/live/filesystem.squashfs"
fi
SQFS_TRAILER_HASH_SIZE=$((SQFS_TOTAL_NEEDED - 8 - SQFS_DATA_SIZE))
python3 -c "import struct,sys; sys.stdout.buffer.write(struct.pack('<Q', $SQFS_TRAILER_HASH_SIZE))" >> "$OUTPUT_DIR/live/filesystem.squashfs"
echo "    squashfs data size:  $SQFS_DATA_SIZE"
echo "    squashfs total size: $SQFS_TOTAL_NEEDED (sector-aligned)"
echo "    squashfs blob (sqfs+verity): $(du -h "$OUTPUT_DIR/live/filesystem.squashfs" | cut -f1)"

echo "$LIVE_ROOTHASH" > "$OUTPUT_DIR/live/verity-roothash"

# Clean up working directory
rm -rf "$WORK_DIR"
WORK_DIR=""

trap - EXIT

echo ""
echo "=============================="
echo "Live rootfs built successfully"
echo "=============================="
echo "Output: $OUTPUT_DIR"
echo "  vmlinuz:              $(du -h "$OUTPUT_DIR/live/vmlinuz" | cut -f1)"
echo "  initrd.img:           $(du -h "$OUTPUT_DIR/live/initrd.img" | cut -f1)"
echo "  filesystem.squashfs:  $(du -h "$OUTPUT_DIR/live/filesystem.squashfs" | cut -f1)"
echo "  verity root hash:     $LIVE_ROOTHASH"
echo "=============================="
