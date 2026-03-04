#!/bin/bash
#
# Container-based installer integration test: extract the live rootfs from
# a built ISO, create a loopback block device, and run the real installer
# inside a systemd-nspawn container targeting that loop device.
#
# This tests the full write + partition-expand + firstboot pipeline without
# booting a VM, using the exact same rootfs that ships in the live ISO.
#
# Usage: test-container-install.sh <iso> <variant> [arch]
#   variant: metal | cloud
#   arch:    amd64 | arm64 (default: amd64)
#
# Requires: systemd-nspawn, xorriso, unsquashfs, losetup, lsblk, cryptsetup,
#           btrfs-progs, util-linux. Must run as root.
set -euo pipefail

ISO="${1:?Usage: $0 <iso> <variant> [arch]}"
VARIANT="${2:?Usage: $0 <iso> <variant> [arch]}"
ARCH="${3:-amd64}"

TARGET_DISK_SIZE="${TARGET_DISK_SIZE:-16G}"

if [ ! -f "$ISO" ]; then
    echo "ERROR: ISO not found: $ISO"
    exit 1
fi

case "$VARIANT" in
    metal|cloud) ;;
    *)
        echo "ERROR: variant must be metal or cloud (got: $VARIANT)"
        exit 1
        ;;
esac

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
# State tracking for cleanup
# ============================================================
WORK_DIR=""
LOOP_DEV=""
LUKS_NAME="bes-container-test-root"
VERIFY_MOUNT=""

cleanup() {
    local exit_code=$?
    set +e

    if [ -n "$VERIFY_MOUNT" ] && mountpoint -q "$VERIFY_MOUNT" 2>/dev/null; then
        umount "$VERIFY_MOUNT"
    fi

    if [ -e "/dev/mapper/$LUKS_NAME" ]; then
        cryptsetup close "$LUKS_NAME" 2>/dev/null
    fi

    if [ -n "$LOOP_DEV" ]; then
        losetup -d "$LOOP_DEV" 2>/dev/null
    fi

    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi

    if [ "$exit_code" -ne 0 ]; then
        echo ""
        echo "!!! Container install test FAILED (exit code $exit_code)"
    fi

    exit "$exit_code"
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-container-test-XXXXXX)"

echo "=============================="
echo "BES Container Install Test"
echo "=============================="
echo "ISO:         $ISO"
echo "Variant:     $VARIANT"
echo "Arch:        $ARCH"
echo "Disk size:   $TARGET_DISK_SIZE"
echo "Work dir:    $WORK_DIR"
echo "=============================="
echo ""

# ============================================================
# Phase 1: Extract rootfs and images from ISO
# ============================================================

echo "==> Phase 1: Extracting squashfs and images from ISO..."

SQUASHFS="$WORK_DIR/filesystem.squashfs"
ROOTFS="$WORK_DIR/rootfs"
IMAGES_STAGING="$WORK_DIR/images"

xorriso -osirrox on -indev "$ISO" \
    -extract /live/filesystem.squashfs "$SQUASHFS" \
    2>/dev/null

if [ ! -f "$SQUASHFS" ]; then
    echo "ERROR: failed to extract /live/filesystem.squashfs from ISO"
    exit 1
fi

echo "    Extracted squashfs: $(du -h "$SQUASHFS" | cut -f1)"

unsquashfs -d "$ROOTFS" -f "$SQUASHFS" >/dev/null 2>&1
rm -f "$SQUASHFS"
echo "    Unpacked rootfs to $ROOTFS"

mkdir -p "$IMAGES_STAGING"
xorriso -osirrox on -indev "$ISO" \
    -extract /images "$IMAGES_STAGING" \
    2>/dev/null

IMAGE_COUNT=$(find "$IMAGES_STAGING" -name '*.raw.zst' | wc -l)
if [ "$IMAGE_COUNT" -eq 0 ]; then
    echo "ERROR: no .raw.zst images found in ISO /images/"
    exit 1
fi
echo "    Extracted $IMAGE_COUNT disk image(s)"

# ============================================================
# Phase 2: Create loopback target disk
# ============================================================
# r[verify installer.tui.loop-device]
echo "==> Phase 2: Creating loopback target disk ($TARGET_DISK_SIZE)..."

TARGET_IMG="$WORK_DIR/target.img"
truncate -s "$TARGET_DISK_SIZE" "$TARGET_IMG"

LOOP_DEV="$(losetup --show --find --partscan "$TARGET_IMG")"
echo "    Loop device: $LOOP_DEV"

# Get the size in bytes for the fake-devices JSON
LOOP_SIZE_BYTES=$(blockdev --getsize64 "$LOOP_DEV")
echo "    Size: $LOOP_SIZE_BYTES bytes"

# ============================================================
# Phase 3: Prepare installer inputs
# ============================================================
echo "==> Phase 3: Preparing installer configuration..."

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
echo "    devices.json: $DEVICES_JSON"

TAILSCALE_TEST_KEY="tskey-auth-container-test-key-1234567890"

CONFIG_TOML="$WORK_DIR/bes-install.toml"
cat > "$CONFIG_TOML" << EOF
auto = true
variant = "$VARIANT"
disk = "$LOOP_DEV"
disable-tpm = true

[firstboot]
hostname = "container-test-$VARIANT"
tailscale-authkey = "$TAILSCALE_TEST_KEY"
ssh-authorized-keys = [
  "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestKeyForContainerInstallTest test@container",
]
EOF
echo "    Config:"
sed 's/^/      /' "$CONFIG_TOML"

# Ensure the rootfs has os-release (systemd-nspawn requires it)
if [ ! -f "$ROOTFS/etc/os-release" ] && [ ! -f "$ROOTFS/usr/lib/os-release" ]; then
    echo "BES Installer Live" > "$ROOTFS/etc/os-release"
fi

# ============================================================
# Phase 4: Run installer inside systemd-nspawn
# ============================================================
# r[verify installer.container.isolation]
echo "==> Phase 4: Running installer in systemd-nspawn container..."

# Build the list of bind mounts for loop device + partition nodes.
# Pre-create partition device nodes: after the installer writes the image
# and calls partprobe, the kernel creates /dev/loopNpM on the host.
# But inside nspawn /dev is private, so we must bind them in.
# Since partitions don't exist yet, we bind the loop device now and also
# set up the partition nodes to be bound once they appear.
#
# Strategy: we bind the whole loop device in. For partition devices that
# appear after writing, we pre-create them outside and bind them in.
# The installer calls partprobe which updates the host kernel's view,
# and the bind-mounted nodes will then work.

# We expect 3 partitions (EFI, xboot, root) after writing the image.
# Pre-create placeholder files so the kernel has somewhere to put the
# partition devices once partprobe runs.
for i in 1 2 3; do
    PART_DEV="${LOOP_DEV}p${i}"
    if [ ! -e "$PART_DEV" ]; then
        # Touch a placeholder file so --bind has something to mount on.
        # It will become functional once partprobe creates the real device.
        touch "$PART_DEV.placeholder"
    fi
done

# We need the partition nodes to be visible inside the container after
# partprobe runs. The simplest reliable approach: bind-mount the host's
# /dev/loopN* into the container's /dev/ by overlaying specific paths.
NSPAWN_BINDS=(
    "--bind=$LOOP_DEV"
)

# Bind-mount images into the path the installer searches
NSPAWN_BINDS+=("--bind-ro=$IMAGES_STAGING:/run/live/medium/images")

# Bind-mount the config where the installer expects it
NSPAWN_BINDS+=("--bind-ro=$CONFIG_TOML:/run/besconf/bes-install.toml")

# Bind-mount the devices JSON
NSPAWN_BINDS+=("--bind-ro=$DEVICES_JSON:/tmp/devices.json")

# Bind /dev so that partition devices appear after partprobe.
# This is necessary because nspawn's private /dev won't see new nodes
# created by the host kernel. Using --bind=/dev exposes host devices,
# but the --fake-devices flag ensures the installer can't discover them.
# Combined with the config targeting only the loop device, this is safe.
NSPAWN_BINDS+=("--bind=/dev")

INSTALLER_LOG="$WORK_DIR/installer.log"

echo "    Running installer (variant=$VARIANT, target=$LOOP_DEV)..."
echo ""

# Run the installer. Use --pipe for non-interactive output.
# --register=no avoids needing systemd-machined.
# --quiet suppresses nspawn's own status messages.
set +e
systemd-nspawn \
    --register=no \
    --quiet \
    --pipe \
    --directory="$ROOTFS" \
    --private-network \
    "${NSPAWN_BINDS[@]}" \
    /usr/local/bin/bes-installer \
        --fake-devices /tmp/devices.json \
        --config /run/besconf/bes-install.toml \
        --log /tmp/installer.log \
        --no-reboot \
    2>&1
INSTALLER_RC=$?
set -e

# Copy out the log if it exists
if [ -f "$ROOTFS/tmp/installer.log" ]; then
    cp "$ROOTFS/tmp/installer.log" "$INSTALLER_LOG"
fi

echo ""
if [ $INSTALLER_RC -ne 0 ]; then
    echo "!!! Installer exited with code $INSTALLER_RC"
    if [ -f "$INSTALLER_LOG" ]; then
        echo ""
        echo "Installer log:"
        cat "$INSTALLER_LOG"
    fi
    exit 1
fi

echo "    Installer exited successfully."

# ============================================================
# Phase 5: Verify the written disk
# ============================================================

echo "==> Phase 5: Verifying written disk..."

PASS=0
FAIL=0
ERRORS=()

check() {
    local desc="$1"; shift
    if "$@" >/dev/null 2>&1; then
        echo "    PASS: $desc"
        ((PASS++))
    else
        echo "    FAIL: $desc"
        ERRORS+=("$desc")
        ((FAIL++))
    fi
}

# Re-read partition table on the host
partprobe "$LOOP_DEV"
sleep 1

# Verify partitions exist
check "partition 1 (EFI) exists" test -b "${LOOP_DEV}p1"
check "partition 2 (xboot) exists" test -b "${LOOP_DEV}p2"
check "partition 3 (root) exists" test -b "${LOOP_DEV}p3"

# Verify partition labels via lsblk
LSBLK_JSON=$(lsblk --json --output NAME,PARTLABEL "$LOOP_DEV" 2>/dev/null || echo '{}')
check "EFI partition label present" test -n "$(echo "$LSBLK_JSON" | grep "efi")"
check "xboot partition label present" test -n "$(echo "$LSBLK_JSON" | grep "xboot")"
check "root partition label present" test -n "$(echo "$LSBLK_JSON" | grep "root")"

# r[verify installer.write.partitions]
# Verify that partition 3 (root) was expanded beyond the original image size.
# The raw image is 8 GiB and the target disk is 16 GiB, so the root partition
# must be larger than the original (~7.5 GiB after EFI + xboot partitions).
ROOT_PART_SIZE=$(lsblk --bytes --noheadings --output SIZE "${LOOP_DEV}p3" 2>/dev/null | tr -d '[:space:]')
ROOT_PART_SIZE="${ROOT_PART_SIZE:-0}"
# 8 GiB in bytes = 8589934592; root partition should be well above this after expansion
IMAGE_RAW_SIZE=8589934592
echo "    Root partition size: $ROOT_PART_SIZE bytes (image was $IMAGE_RAW_SIZE bytes)"
check "root partition expanded beyond image size" test "$ROOT_PART_SIZE" -gt "$IMAGE_RAW_SIZE"

# Mount and verify first-boot configuration
VERIFY_MOUNT="$WORK_DIR/verify-mount"
mkdir -p "$VERIFY_MOUNT"

ROOT_PART="${LOOP_DEV}p3"

if [ "$VARIANT" = "metal" ]; then
    echo "    Opening LUKS volume..."
    EMPTY_KEYFILE="$WORK_DIR/empty-keyfile"
    touch "$EMPTY_KEYFILE"
    chmod 400 "$EMPTY_KEYFILE"

    set +e
    cryptsetup open "$ROOT_PART" "$LUKS_NAME" --key-file "$EMPTY_KEYFILE" 2>/dev/null
    LUKS_RC=$?
    set -e

    if [ $LUKS_RC -eq 0 ]; then
        check "LUKS volume opened with empty keyfile" true
        BTRFS_DEV="/dev/mapper/$LUKS_NAME"
    else
        check "LUKS volume opened with empty keyfile" false
        echo "    Cannot open LUKS volume; skipping filesystem checks."
        BTRFS_DEV=""
    fi
else
    BTRFS_DEV="$ROOT_PART"
fi

if [ -n "$BTRFS_DEV" ]; then
    set +e
    mount -t btrfs -o subvol=@,ro "$BTRFS_DEV" "$VERIFY_MOUNT" 2>/dev/null
    MOUNT_RC=$?
    set -e

    if [ $MOUNT_RC -eq 0 ]; then
        check "btrfs root mounted successfully" true

        # r[verify installer.firstboot.hostname]
        # Hostname: /etc/hostname content and /etc/hosts entry
        if [ -f "$VERIFY_MOUNT/etc/hostname" ]; then
            ACTUAL_HOSTNAME="$(tr -d '[:space:]' < "$VERIFY_MOUNT/etc/hostname")"
            check "hostname is container-test-$VARIANT" \
                test "$ACTUAL_HOSTNAME" = "container-test-$VARIANT"
        else
            check "hostname file exists" false
        fi

        if [ -f "$VERIFY_MOUNT/etc/hosts" ]; then
            check "/etc/hosts contains 127.0.1.1 entry for hostname" \
                grep -q "127.0.1.1.*container-test-$VARIANT" "$VERIFY_MOUNT/etc/hosts"
        else
            check "/etc/hosts file exists" false
        fi

        # r[verify installer.firstboot.tailscale-authkey]
        # Tailscale authkey: file exists, contains key, has 600 perms
        TS_KEY_FILE="$VERIFY_MOUNT/etc/bes/tailscale-authkey"
        if [ -f "$TS_KEY_FILE" ]; then
            check "tailscale-authkey contains configured key" \
                grep -q "$TAILSCALE_TEST_KEY" "$TS_KEY_FILE"

            TS_PERMS=$(stat -c '%a' "$TS_KEY_FILE")
            check "tailscale-authkey permissions are 600" test "$TS_PERMS" = "600"
        else
            check "tailscale-authkey file exists" false
        fi

        # r[verify installer.firstboot.ssh-keys]
        # SSH authorized keys: file exists, contains key, correct perms
        AK_FILE="$VERIFY_MOUNT/home/ubuntu/.ssh/authorized_keys"
        if [ -f "$AK_FILE" ]; then
            check "authorized_keys contains test key" \
                grep -q "TestKeyForContainerInstallTest" "$AK_FILE"

            AK_PERMS=$(stat -c '%a' "$AK_FILE")
            check "authorized_keys permissions are 600" test "$AK_PERMS" = "600"

            SSH_DIR_PERMS=$(stat -c '%a' "$VERIFY_MOUNT/home/ubuntu/.ssh")
            check ".ssh directory permissions are 700" test "$SSH_DIR_PERMS" = "700"
        else
            check "authorized_keys file exists" false
        fi

        # r[verify installer.firstboot.tpm-disable]
        # TPM disable check (metal only)
        if [ "$VARIANT" = "metal" ]; then
            TPM_SYMLINK="$VERIFY_MOUNT/etc/systemd/system/multi-user.target.wants/setup-tpm-unlock.service"
            check "setup-tpm-unlock.service symlink removed" test ! -e "$TPM_SYMLINK"
        fi

        umount "$VERIFY_MOUNT"
        VERIFY_MOUNT=""
    else
        check "btrfs root mounted successfully" false
        echo "    Cannot mount btrfs root; skipping file checks."
    fi

    if [ "$VARIANT" = "metal" ] && [ -e "/dev/mapper/$LUKS_NAME" ]; then
        cryptsetup close "$LUKS_NAME"
    fi
fi

# ============================================================
# Phase 6: Results
# ============================================================
echo ""
echo "=============================="
echo "Container Install Test Results ($VARIANT / $ARCH)"
echo "=============================="
echo "  $PASS passed, $FAIL failed"

if [ ${#ERRORS[@]} -gt 0 ]; then
    echo ""
    echo "  Failures:"
    for e in "${ERRORS[@]}"; do
        echo "    - $e"
    done
fi

if [ -f "$INSTALLER_LOG" ]; then
    echo ""
    echo "  Installer log: $INSTALLER_LOG"
fi

echo ""

if [ "$FAIL" -eq 0 ]; then
    echo "Container install test PASSED"
    exit 0
else
    echo "Container install test FAILED"
    exit 1
fi
