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
for cmd in systemd-nspawn xorriso unsquashfs losetup lsblk partprobe jq; do
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
SCENARIOS_JSON="$SCRIPT_DIR/container-install-scenarios.json"
if [ ! -f "$SCENARIOS_JSON" ]; then
    echo "ERROR: scenario definitions not found: $SCENARIOS_JSON"
    exit 1
fi

# Helper: read a string field from a scenario JSON object, defaulting to "".
jq_str() { echo "$1" | jq -r "$2 // empty"; }

# ============================================================
# Run scenarios
# ============================================================
TOTAL=$(jq 'length' "$SCENARIOS_JSON")
PASSED=0
FAILED=0
FAILED_NAMES=()

echo "=============================="
echo "Running $TOTAL scenarios"
echo "=============================="
echo ""

for i in $(seq 0 $((TOTAL - 1))); do
    SCENARIO=$(jq -c ".[$i]" "$SCENARIOS_JSON")

    name=$(jq_str "$SCENARIO" '.name')
    variant=$(jq_str "$SCENARIO" '.variant')
    disable_tpm=$(echo "$SCENARIO" | jq -r '."disable-tpm" // true')

    SCENARIO_NUM=$((i + 1))
    echo "[$SCENARIO_NUM/$TOTAL] $name"

    set +e
    SCENARIO_NAME="$name" \
    ROOTFS_DIR="$ROOTFS_DIR" \
    IMAGES_DIR="$IMAGES_DIR" \
    DISABLE_TPM="$disable_tpm" \
    SET_HOSTNAME="$(jq_str "$SCENARIO" '.hostname')" \
    SET_HOSTNAME_FROM_DHCP="$(jq_str "$SCENARIO" '."hostname-from-dhcp"')" \
    SET_HOSTNAME_TEMPLATE="$(jq_str "$SCENARIO" '."hostname-template"')" \
    SET_HOSTNAME_TEMPLATE_REGEX="$(jq_str "$SCENARIO" '."hostname-template-regex"')" \
    SET_TAILSCALE="$(jq_str "$SCENARIO" '.tailscale')" \
    SET_SSH_KEYS="$(jq_str "$SCENARIO" '."ssh-keys"')" \
    SET_PASSWORD="$(jq_str "$SCENARIO" '.password')" \
    SET_PASSWORD_HASH="$(jq_str "$SCENARIO" '."password-hash"')" \
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
