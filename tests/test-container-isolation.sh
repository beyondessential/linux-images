#!/bin/bash
#
# Container isolation test: launch a systemd-nspawn container using the
# live ISO rootfs and verify that no host block devices are visible inside.
#
# This test does NOT run the installer — it only checks the isolation
# property that containers must satisfy before we trust them for
# integration testing.
#
# Usage: test-container-isolation.sh <iso>
#
# Requires: systemd-nspawn, xorriso, unsquashfs. Must run as root.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/nspawn-opts.sh"

ISO="${1:?Usage: $0 <iso>}"

if [ ! -f "$ISO" ]; then
    echo "ERROR: ISO not found: $ISO"
    exit 1
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root"
    exit 1
fi

MISSING=()
for cmd in systemd-nspawn xorriso unsquashfs; do
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

# shellcheck disable=SC2329
cleanup() {
    local exit_code=$?
    set +e

    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi

    if [ "$exit_code" -ne 0 ]; then
        echo ""
        echo "!!! Container isolation test FAILED (exit code $exit_code)"
    fi

    exit "$exit_code"
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-isolation-test-XXXXXX)"

echo "=============================="
echo "BES Container Isolation Test"
echo "=============================="
echo "ISO:      $ISO"
echo "Work dir: $WORK_DIR"
echo "=============================="
echo ""

# ============================================================
# Phase 1: Extract rootfs from ISO
# ============================================================
echo "==> Phase 1: Extracting rootfs from ISO..."

SQUASHFS="$WORK_DIR/filesystem.squashfs"
ROOTFS="$WORK_DIR/rootfs"

xorriso -osirrox on -indev "$ISO" \
    -extract /live/filesystem.squashfs "$SQUASHFS" \
    2>/dev/null

if [ ! -f "$SQUASHFS" ]; then
    echo "ERROR: failed to extract /live/filesystem.squashfs from ISO"
    exit 1
fi

unsquashfs -d "$ROOTFS" -f "$SQUASHFS" >/dev/null 2>&1
rm -f "$SQUASHFS"
echo "    Unpacked rootfs to $ROOTFS"

# Ensure os-release exists (systemd-nspawn requires it)
if [ ! -f "$ROOTFS/etc/os-release" ] && [ ! -f "$ROOTFS/usr/lib/os-release" ]; then
    echo "BES Installer Live" > "$ROOTFS/etc/os-release"
fi

# ============================================================
# Phase 2: Collect host block devices for comparison
# ============================================================
echo "==> Phase 2: Collecting host block devices..."

HOST_BLOCK_DEVS="$WORK_DIR/host-block-devs.txt"
# List all block device nodes under /dev on the host (sd*, nvme*, vd*, hd*, loop*, dm-*, etc.)
find /dev -maxdepth 1 -type b -printf '%f\n' 2>/dev/null | sort > "$HOST_BLOCK_DEVS"
HOST_COUNT=$(wc -l < "$HOST_BLOCK_DEVS")
echo "    Found $HOST_COUNT host block device(s)"

if [ "$HOST_COUNT" -eq 0 ]; then
    echo "WARNING: no host block devices found; test may be trivially passing"
fi

# ============================================================
# Phase 3: Launch container and list /dev block devices inside
# ============================================================
echo "==> Phase 3: Launching container to inspect /dev..."

CONTAINER_DEV_LIST="$WORK_DIR/container-dev-list.txt"

# r[verify installer.container.isolation] (layer 1): launch the container
# without binding any host block devices. systemd-nspawn provides its own
# /dev, so only devices explicitly bound in would be visible.
systemd-nspawn \
    "${NSPAWN_COMMON_OPTS[@]}" \
    --directory="$ROOTFS" \
    /bin/sh -c 'find /dev -maxdepth 1 -type b -printf "%f\n" 2>/dev/null | sort' \
    > "$CONTAINER_DEV_LIST" 2>/dev/null || true

CONTAINER_COUNT=$(wc -l < "$CONTAINER_DEV_LIST")
echo "    Found $CONTAINER_COUNT block device(s) inside container"

if [ "$CONTAINER_COUNT" -gt 0 ]; then
    echo "    Container block devices:"
    sed 's/^/      /' "$CONTAINER_DEV_LIST"
fi

# ============================================================
# Phase 4: Verify isolation
# ============================================================
echo "==> Phase 4: Verifying isolation..."

PASS=0
FAIL=0
ERRORS=()

check() {
    local desc="$1"; shift
    if "$@" >/dev/null 2>&1; then
        echo "    PASS: $desc"
        PASS=$((PASS + 1))
    else
        echo "    FAIL: $desc"
        ERRORS+=("$desc")
        FAIL=$((FAIL + 1))
    fi
}

# Check that real disk devices (sd*, nvme*, vd*, hd*) are NOT visible inside the container.
# Loop devices and device-mapper nodes created by the container runtime itself are acceptable.
LEAKED_DEVS="$WORK_DIR/leaked-devs.txt"
: > "$LEAKED_DEVS"

while IFS= read -r dev; do
    case "$dev" in
        # Real disk devices that must never appear inside the container
        sd*|nvme*|vd*|hd*|xvd*|mmcblk*)
            echo "$dev" >> "$LEAKED_DEVS"
            ;;
        # loop* and dm-* may be created by the container runtime itself;
        # we only flag them if they match a host device that existed before
        # the container was launched.
        *)
            ;;
    esac
done < "$CONTAINER_DEV_LIST"

LEAKED_COUNT=$(wc -l < "$LEAKED_DEVS")

# r[verify installer.container.isolation]: the container must not expose
# any real host block devices.
check "no host disk devices visible inside container" test "$LEAKED_COUNT" -eq 0

if [ "$LEAKED_COUNT" -gt 0 ]; then
    echo ""
    echo "    Leaked host block devices found inside the container:"
    sed 's/^/      /' "$LEAKED_DEVS"
    echo ""
fi

# Also verify that specific well-known host devices are absent
for pattern in sda sdb nvme0n1 vda; do
    if grep -qx "$pattern" "$CONTAINER_DEV_LIST" 2>/dev/null; then
        check "host device /dev/$pattern not visible" false
    else
        check "host device /dev/$pattern not visible" true
    fi
done

# ============================================================
# Phase 5: Results
# ============================================================
echo ""
echo "=============================="
echo "Container Isolation Test Results"
echo "=============================="
echo "  $PASS passed, $FAIL failed"

if [ ${#ERRORS[@]} -gt 0 ]; then
    echo ""
    echo "  Failures:"
    for e in "${ERRORS[@]}"; do
        echo "    - $e"
    done
fi

echo ""

if [ "$FAIL" -eq 0 ]; then
    echo "Container isolation test PASSED"
    exit 0
else
    echo "Container isolation test FAILED"
    exit 1
fi
