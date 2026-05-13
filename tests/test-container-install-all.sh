#!/bin/bash
#
# Driver for container-based installer integration tests.
# Extracts the ISO once, then runs multiple scenarios against it.
#
# Usage: test-container-install-all.sh <iso> <arch> [filter]
#   arch: amd64 | arm64
#   filter: one of the following (omit to run all scenarios):
#     "encrypted"          — run only encrypted scenarios (tpm/keyfile)
#     "plain"              — run only plain (unencrypted) scenarios
#     "shard:I/N"          — run shard I of N (1-indexed, round-robin over
#                            scenarios sorted by name)
#     "<scenario-name>"    — run a single scenario by exact name
#     "<substring>"        — run all scenarios whose name contains the string
#
# Each scenario runs test-container-install.sh with different environment
# variables controlling disk-encryption, hostname, tailscale, and SSH keys.
#
# Requires: systemd-nspawn, xorriso, unsquashfs, losetup, lsblk, partprobe,
#           cryptsetup, btrfs-progs, util-linux, veritysetup, sgdisk.
#           Must run as root.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/iso-images-mount.sh"

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
for cmd in systemd-nspawn xorriso unsquashfs losetup lsblk partprobe jq sgdisk veritysetup; do
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

# shellcheck disable=SC2329 # invoked indirectly via trap
cleanup() {
    local exit_code=$?
    set +e
    iso_images_cleanup
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
# Scenario definitions
# ============================================================
SCENARIOS_JSON="$SCRIPT_DIR/container-install-scenarios.json"
if [ ! -f "$SCENARIOS_JSON" ]; then
    echo "ERROR: scenario definitions not found: $SCENARIOS_JSON"
    exit 1
fi

# Sort by name so the run order — and any shard partitioning — is stable
# regardless of how scenarios are arranged in the source JSON.
SCENARIOS_SORTED=$(jq -c 'sort_by(.name)' "$SCENARIOS_JSON")

# Helper: read a string field from a scenario JSON object, defaulting to "".
jq_str() { echo "$1" | jq -r "$2 // empty"; }

# Apply filter if specified. Accepts category names ("encrypted"/"plain"),
# a shard spec ("shard:I/N"), an exact scenario name, or a substring match.
if [ -n "$FILTER" ]; then
    case "$FILTER" in
        encrypted|metal)
            SCENARIOS_FILTERED=$(echo "$SCENARIOS_SORTED" | jq -c '[.[] | select(."disk-encryption" == "tpm" or ."disk-encryption" == "keyfile")]')
            ;;
        plain|cloud)
            SCENARIOS_FILTERED=$(echo "$SCENARIOS_SORTED" | jq -c '[.[] | select(."disk-encryption" == "none")]')
            ;;
        shard:*)
            SPEC="${FILTER#shard:}"
            SHARD_INDEX="${SPEC%/*}"
            SHARD_COUNT="${SPEC#*/}"
            if ! [[ "$SHARD_INDEX" =~ ^[0-9]+$ ]] || ! [[ "$SHARD_COUNT" =~ ^[0-9]+$ ]]; then
                echo "ERROR: shard filter must be 'shard:I/N' with integers (got: $FILTER)"
                exit 1
            fi
            if [ "$SHARD_COUNT" -lt 1 ] || [ "$SHARD_INDEX" -lt 1 ] || [ "$SHARD_INDEX" -gt "$SHARD_COUNT" ]; then
                echo "ERROR: shard index $SHARD_INDEX out of range for $SHARD_COUNT shards"
                exit 1
            fi
            # Round-robin: scenario at sorted index k goes to shard (k mod N) + 1.
            # This spreads slow scenarios (e.g. luks-tpm-swtpm) across shards
            # rather than concentrating them in one.
            SCENARIOS_FILTERED=$(echo "$SCENARIOS_SORTED" | jq -c \
                --argjson i "$((SHARD_INDEX - 1))" \
                --argjson n "$SHARD_COUNT" \
                '[to_entries[] | select(.key % $n == $i) | .value]')
            ;;
        *)
            # Try exact name match first, then substring match.
            SCENARIOS_FILTERED=$(echo "$SCENARIOS_SORTED" | jq -c --arg f "$FILTER" '[.[] | select(.name == $f)]')
            if [ "$(echo "$SCENARIOS_FILTERED" | jq 'length')" -eq 0 ]; then
                SCENARIOS_FILTERED=$(echo "$SCENARIOS_SORTED" | jq -c --arg f "$FILTER" '[.[] | select(.name | contains($f))]')
            fi
            if [ "$(echo "$SCENARIOS_FILTERED" | jq 'length')" -eq 0 ]; then
                echo "ERROR: no scenarios match filter '$FILTER'"
                exit 1
            fi
            ;;
    esac
else
    SCENARIOS_FILTERED="$SCENARIOS_SORTED"
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
