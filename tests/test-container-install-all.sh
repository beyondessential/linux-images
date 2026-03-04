#!/bin/bash
#
# Driver for container-based installer integration tests.
# Extracts the ISO once, then runs multiple scenarios against it.
#
# Usage: test-container-install-all.sh <iso> <arch>
#   arch: amd64 | arm64
#
# Each scenario runs test-container-install.sh with different environment
# variables controlling variant, TPM, hostname, tailscale, and SSH keys.
#
# Requires: systemd-nspawn, xorriso, unsquashfs, losetup, lsblk, partprobe,
#           cryptsetup, btrfs-progs, util-linux. Must run as root.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ISO="${1:?Usage: $0 <iso> <arch>}"
ARCH="${2:?Usage: $0 <iso> <arch>}"

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
for cmd in systemd-nspawn xorriso unsquashfs losetup lsblk partprobe; do
    command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
done
if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "ERROR: missing required commands: ${MISSING[*]}"
    exit 1
fi

# ============================================================
# Shared state
# ============================================================
WORK_DIR=""

cleanup() {
    local exit_code=$?
    set +e
    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi
    exit "$exit_code"
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-container-test-all-XXXXXX)"

echo "=============================="
echo "BES Container Install Test Suite"
echo "=============================="
echo "ISO:       $ISO"
echo "Arch:      $ARCH"
echo "Work dir:  $WORK_DIR"
echo "=============================="
echo ""

# ============================================================
# Phase 1: Extract rootfs and images from ISO (once)
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
# Scenario definitions
# ============================================================
# Each scenario is a tab-separated line:
#   name | variant | disable_tpm | hostname | tailscale_key | ssh_key
#
# Empty string means "not set" for optional fields.

SSH_TEST_KEY="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestKeyForContainerInstallTest test@container"
TS_TEST_KEY="tskey-auth-container-test-key-1234567890"

SCENARIOS=(
    # 1. Metal, full firstboot, TPM disabled
    "metal-full-disable-tpm|metal|true|test-metal-full|$TS_TEST_KEY|$SSH_TEST_KEY"

    # 2. Metal, no firstboot, TPM enabled
    "metal-minimal-tpm-on|metal|false|||"

    # 3. Metal, hostname only, TPM disabled
    "metal-hostname-only|metal|true|test-metal-hostname||"

    # 4. Cloud, full firstboot
    "cloud-full|cloud|true|test-cloud-full|$TS_TEST_KEY|$SSH_TEST_KEY"

    # 5. Cloud, no firstboot
    "cloud-minimal|cloud|true|||"

    # 6. Cloud, tailscale only (no hostname, no SSH)
    "cloud-tailscale-only|cloud|true||$TS_TEST_KEY|"
)

# ============================================================
# Run scenarios
# ============================================================
TOTAL=${#SCENARIOS[@]}
PASSED=0
FAILED=0
FAILED_NAMES=()

echo "=============================="
echo "Running $TOTAL scenarios"
echo "=============================="
echo ""

for i in "${!SCENARIOS[@]}"; do
    IFS='|' read -r name variant disable_tpm hostname ts_key ssh_key <<< "${SCENARIOS[$i]}"

    SCENARIO_NUM=$((i + 1))
    echo "[$SCENARIO_NUM/$TOTAL] $name"

    set +e
    SCENARIO_NAME="$name" \
    ROOTFS_DIR="$ROOTFS_DIR" \
    IMAGES_DIR="$IMAGES_DIR" \
    DISABLE_TPM="$disable_tpm" \
    SET_HOSTNAME="$hostname" \
    SET_TAILSCALE="$ts_key" \
    SET_SSH_KEYS="$ssh_key" \
        "$SCRIPT_DIR/test-container-install.sh" "$variant" "$ARCH"
    RC=$?
    set -e

    if [ $RC -eq 0 ]; then
        PASSED=$((PASSED + 1))
    else
        FAILED=$((FAILED + 1))
        FAILED_NAMES+=("$name")
    fi

    echo ""
done

# ============================================================
# Summary
# ============================================================
echo "=============================="
echo "Test Suite Summary"
echo "=============================="
echo "  Total:   $TOTAL"
echo "  Passed:  $PASSED"
echo "  Failed:  $FAILED"

if [ ${#FAILED_NAMES[@]} -gt 0 ]; then
    echo ""
    echo "  Failed scenarios:"
    for name in "${FAILED_NAMES[@]}"; do
        echo "    - $name"
    done
fi

echo "=============================="
echo ""

if [ "$FAILED" -eq 0 ]; then
    echo "All scenarios passed."
    exit 0
else
    echo "Some scenarios failed."
    exit 1
fi
