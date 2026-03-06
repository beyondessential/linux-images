#!/bin/bash
#
# Driver for container-based installer integration tests.
# Extracts the ISO once, then runs multiple scenarios against it.
#
# Usage: test-container-install-all.sh <iso> <arch> [filter]
#   arch: amd64 | arm64
#   filter: one of the following (omit to run all scenarios):
#     "metal"              — run only metal-variant scenarios (tpm/keyfile)
#     "cloud"              — run only cloud-variant scenarios (none)
#     "<scenario-name>"    — run a single scenario by exact name
#     "<substring>"        — run all scenarios whose name contains the string
#
# Each scenario runs test-container-install.sh with different environment
# variables controlling disk-encryption, hostname, tailscale, and SSH keys.
#
# Requires: systemd-nspawn, xorriso, unsquashfs, losetup, lsblk, partprobe,
#           cryptsetup, btrfs-progs, util-linux. Must run as root.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ISO="${1:?Usage: $0 <iso> <arch> [filter]}"
ARCH="${2:?Usage: $0 <iso> <arch> [filter]}"
FILTER="${3:-}"

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

if [ ! -f "$IMAGES_DIR/partitions.json" ]; then
    echo "ERROR: partitions.json not found in ISO /images/"
    exit 1
fi
PART_IMAGE_COUNT=$(find "$IMAGES_DIR" -name '*.img.zst' | wc -l)
echo "    Extracted partitions.json + $PART_IMAGE_COUNT partition image(s)"
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

# Apply filter if specified. Accepts variant names ("metal"/"cloud"),
# an exact scenario name, or a substring match.
if [ -n "$FILTER" ]; then
    case "$FILTER" in
        metal)
            SCENARIOS_FILTERED=$(jq -c '[.[] | select(."disk-encryption" == "tpm" or ."disk-encryption" == "keyfile")]' "$SCENARIOS_JSON")
            ;;
        cloud)
            SCENARIOS_FILTERED=$(jq -c '[.[] | select(."disk-encryption" == "none")]' "$SCENARIOS_JSON")
            ;;
        *)
            # Try exact name match first, then substring match.
            SCENARIOS_FILTERED=$(jq -c --arg f "$FILTER" '[.[] | select(.name == $f)]' "$SCENARIOS_JSON")
            if [ "$(echo "$SCENARIOS_FILTERED" | jq 'length')" -eq 0 ]; then
                SCENARIOS_FILTERED=$(jq -c --arg f "$FILTER" '[.[] | select(.name | contains($f))]' "$SCENARIOS_JSON")
            fi
            if [ "$(echo "$SCENARIOS_FILTERED" | jq 'length')" -eq 0 ]; then
                echo "ERROR: no scenarios match filter '$FILTER'"
                exit 1
            fi
            ;;
    esac
else
    SCENARIOS_FILTERED=$(jq -c '.' "$SCENARIOS_JSON")
fi

# ============================================================
# Run scenarios
# ============================================================
TOTAL=$(echo "$SCENARIOS_FILTERED" | jq 'length')
PASSED=0
FAILED=0
FAILED_NAMES=()

echo "=============================="
echo "Running $TOTAL scenario(s)${FILTER:+ (filter=$FILTER)}"
echo "=============================="
echo ""

for i in $(seq 0 $((TOTAL - 1))); do
    SCENARIO=$(echo "$SCENARIOS_FILTERED" | jq -c ".[$i]")

    name=$(jq_str "$SCENARIO" '.name')
    disk_encryption=$(jq_str "$SCENARIO" '."disk-encryption"')

    SCENARIO_NUM=$((i + 1))
    echo "[$SCENARIO_NUM/$TOTAL] $name"

    # Default PRIVATE_NETWORK to "true" when the scenario does not specify it.
    scenario_private_network="$(jq_str "$SCENARIO" '."private-network"')"
    if [ -z "$scenario_private_network" ]; then
        scenario_private_network="true"
    fi

    set +e
    SCENARIO_NAME="$name" \
    ROOTFS_DIR="$ROOTFS_DIR" \
    IMAGES_DIR="$IMAGES_DIR" \
    SET_HOSTNAME="$(jq_str "$SCENARIO" '.hostname')" \
    SET_HOSTNAME_FROM_DHCP="$(jq_str "$SCENARIO" '."hostname-from-dhcp"')" \
    SET_HOSTNAME_TEMPLATE="$(jq_str "$SCENARIO" '."hostname-template"')" \
    SET_HOSTNAME_TEMPLATE_REGEX="$(jq_str "$SCENARIO" '."hostname-template-regex"')" \
    SET_TAILSCALE="$(jq_str "$SCENARIO" '.tailscale')" \
    SET_SSH_KEYS="$(jq_str "$SCENARIO" '."ssh-keys"')" \
    SET_PASSWORD="$(jq_str "$SCENARIO" '.password')" \
    SET_PASSWORD_HASH="$(jq_str "$SCENARIO" '."password-hash"')" \
    SET_TIMEZONE="$(jq_str "$SCENARIO" '.timezone')" \
    SET_COPY_INSTALL_LOG="$(jq_str "$SCENARIO" '."copy-install-log"')" \
    PRIVATE_NETWORK="$scenario_private_network" \
        "$SCRIPT_DIR/test-container-install.sh" "$disk_encryption" "$ARCH"
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
