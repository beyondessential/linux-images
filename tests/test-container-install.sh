#!/bin/bash
#
# Container-based installer integration test: run the real installer inside
# a systemd-nspawn container targeting a loopback block device, then verify
# the written disk.
#
# This script tests a single scenario. It is normally invoked by
# test-container-install-all.sh, which extracts the ISO once and runs
# multiple scenarios against it.
#
# Required arguments:
#   $1 — disk-encryption (tpm | keyfile | none)
#   $2 — arch (amd64 | arm64)
#
# Required environment:
#   ROOTFS_DIR   — path to pre-extracted live rootfs
#   IMAGES_DIR   — path to directory containing partitions.json and .img.zst images
#
# Scenario environment (all optional):
#   SCENARIO_NAME  — human-readable name (default: "unnamed")
#   SET_HOSTNAME   — hostname to set, or "" to skip (default: "")
#   SET_TAILSCALE  — tailscale authkey to set, or "" to skip (default: "")
#   SET_SSH_KEYS   — SSH public key to set, or "" to skip (default: "")
#   SET_PASSWORD   — plaintext password for ubuntu user, or "" to skip (default: "")
#   SET_PASSWORD_HASH — pre-hashed password for ubuntu user, or "" to skip (default: "")
#   SET_COPY_INSTALL_LOG — "false" to disable install log copy, or "" for default (default: "")
#
# Requires: systemd-nspawn, losetup, lsblk, partprobe, cryptsetup,
#           btrfs-progs, util-linux. Must run as root.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/nspawn-opts.sh"

DISK_ENCRYPTION="${1:?Usage: $0 <disk-encryption> <arch>}"
ARCH="${2:?Usage: $0 <disk-encryption> <arch>}"

# Derive variant from disk-encryption mode
case "$DISK_ENCRYPTION" in
    tpm|keyfile) VARIANT="metal" ;;
    none)        VARIANT="cloud" ;;
    *)
        echo "ERROR: disk-encryption must be tpm, keyfile, or none (got: $DISK_ENCRYPTION)"
        exit 1
        ;;
esac

ROOTFS_DIR="${ROOTFS_DIR:?ROOTFS_DIR must be set}"
IMAGES_DIR="${IMAGES_DIR:?IMAGES_DIR must be set}"

SCENARIO_NAME="${SCENARIO_NAME:-unnamed}"
SET_HOSTNAME="${SET_HOSTNAME:-}"
SET_HOSTNAME_FROM_DHCP="${SET_HOSTNAME_FROM_DHCP:-}"
SET_HOSTNAME_TEMPLATE="${SET_HOSTNAME_TEMPLATE:-}"
SET_TAILSCALE="${SET_TAILSCALE:-}"
SET_SSH_KEYS="${SET_SSH_KEYS:-}"
SET_PASSWORD="${SET_PASSWORD:-}"
SET_PASSWORD_HASH="${SET_PASSWORD_HASH:-}"
SET_TIMEZONE="${SET_TIMEZONE:-}"
SET_COPY_INSTALL_LOG="${SET_COPY_INSTALL_LOG:-}"

TARGET_DISK_SIZE="${TARGET_DISK_SIZE:-10G}"

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

if [ ! -d "$ROOTFS_DIR" ]; then
    echo "ERROR: ROOTFS_DIR does not exist: $ROOTFS_DIR"
    exit 1
fi

if [ ! -d "$IMAGES_DIR" ]; then
    echo "ERROR: IMAGES_DIR does not exist: $IMAGES_DIR"
    exit 1
fi

# ============================================================
# State tracking for cleanup
# ============================================================
WORK_DIR=""
LOOP_DEV=""
LUKS_NAME="bes-container-test-root"
VERIFY_MOUNT=""

# shellcheck disable=SC2329
cleanup() {
    local exit_code=$?
    set +e

    if [ -n "$VERIFY_MOUNT" ] && mountpoint -q "$VERIFY_MOUNT" 2>/dev/null; then
        umount "$VERIFY_MOUNT"
    fi

    # Clean up any mounts the installer left on the target device or its
    # LUKS mapper before closing LUKS / detaching the loop.
    if [ -n "$LOOP_DEV" ]; then
        grep "$LOOP_DEV\|bes-target-root" /proc/mounts 2>/dev/null \
            | awk '{print $2}' | sort -r | while read -r mp; do
            umount "$mp" 2>/dev/null
        done
    fi

    if [ -e /dev/mapper/bes-target-root ]; then
        cryptsetup close bes-target-root 2>/dev/null
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
        echo "!!! Scenario '$SCENARIO_NAME' FAILED (exit code $exit_code)"
    fi

    exit "$exit_code"
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-container-test-XXXXXX)"

echo "----------------------------------------------------------------------"
echo "Scenario: $SCENARIO_NAME"
echo "----------------------------------------------------------------------"
echo "  disk-encrypt:  $DISK_ENCRYPTION (variant: $VARIANT)"
echo "  arch:          $ARCH"
echo "  hostname:      ${SET_HOSTNAME:-(not set)}"
echo "  hostname-dhcp: ${SET_HOSTNAME_FROM_DHCP:-(not set)}"
echo "  hostname-tmpl: ${SET_HOSTNAME_TEMPLATE:-(not set)}"
echo "  tailscale:     ${SET_TAILSCALE:+(key provided)}${SET_TAILSCALE:-(not set)}"
echo "  ssh-keys:      ${SET_SSH_KEYS:+(key provided)}${SET_SSH_KEYS:-(not set)}"
echo "  password:      ${SET_PASSWORD:+(plaintext provided)}${SET_PASSWORD:-(not set)}"
echo "  password-hash: ${SET_PASSWORD_HASH:+(hash provided)}${SET_PASSWORD_HASH:-(not set)}"
echo "  timezone:      ${SET_TIMEZONE:-(not set, defaults to UTC)}"
echo "  copy-log:      ${SET_COPY_INSTALL_LOG:-(not set, defaults to true)}"
echo "  disk size:     $TARGET_DISK_SIZE"
echo ""

# ============================================================
# Phase 1: Create loopback target disk
# ============================================================
# r[impl installer.tui.loop-device]: it's hard to implement a negative
# r[verify installer.tui.loop-device]
echo "==> Creating loopback target disk ($TARGET_DISK_SIZE)..."

TARGET_IMG="$WORK_DIR/target.img"
truncate -s "$TARGET_DISK_SIZE" "$TARGET_IMG"

LOOP_DEV="$(losetup --show --find --partscan "$TARGET_IMG")"
echo "    Loop device: $LOOP_DEV"

LOOP_SIZE_BYTES=$(blockdev --getsize64 "$LOOP_DEV")

# ============================================================
# Phase 2: Generate installer configuration
# ============================================================
echo "==> Generating installer configuration..."

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

CONFIG_TOML="$WORK_DIR/bes-install.toml"
{
    echo 'auto = true'
    echo "disk-encryption = \"$DISK_ENCRYPTION\""
    echo "disk = \"$LOOP_DEV\""
    if [ "$SET_COPY_INSTALL_LOG" = "false" ]; then
        echo "copy-install-log = false"
    fi

    if [ -n "$SET_HOSTNAME" ] || [ -n "$SET_HOSTNAME_FROM_DHCP" ] || [ -n "$SET_HOSTNAME_TEMPLATE" ] || [ -n "$SET_TAILSCALE" ] || [ -n "$SET_SSH_KEYS" ] || [ -n "$SET_PASSWORD" ] || [ -n "$SET_PASSWORD_HASH" ] || [ -n "$SET_TIMEZONE" ]; then
        echo ""
        echo "[firstboot]"
        if [ -n "$SET_HOSTNAME" ]; then
            echo "hostname = \"$SET_HOSTNAME\""
        fi
        if [ -n "$SET_HOSTNAME_FROM_DHCP" ]; then
            echo "hostname-from-dhcp = true"
        fi
        if [ -n "$SET_HOSTNAME_TEMPLATE" ]; then
            echo "hostname-template = \"$SET_HOSTNAME_TEMPLATE\""
        fi
        if [ -n "$SET_TAILSCALE" ]; then
            echo "tailscale-authkey = \"$SET_TAILSCALE\""
        fi
        if [ -n "$SET_SSH_KEYS" ]; then
            echo "ssh-authorized-keys = ["
            echo "  \"$SET_SSH_KEYS\","
            echo "]"
        fi
        if [ -n "$SET_PASSWORD" ]; then
            echo "password = \"$SET_PASSWORD\""
        fi
        if [ -n "$SET_PASSWORD_HASH" ]; then
            echo "password-hash = \"$SET_PASSWORD_HASH\""
        fi
        if [ -n "$SET_TIMEZONE" ]; then
            echo "timezone = \"$SET_TIMEZONE\""
        fi
    fi
} > "$CONFIG_TOML"

echo "    Config:"
sed 's/^/      /' "$CONFIG_TOML"

# ============================================================
# Phase 3: Prepare rootfs overlay
# ============================================================
# We work on a copy of the rootfs so multiple scenarios don't interfere.
echo "==> Preparing rootfs overlay..."

SCENARIO_ROOTFS="$WORK_DIR/rootfs"
cp -a "$ROOTFS_DIR" "$SCENARIO_ROOTFS"

if [ ! -f "$SCENARIO_ROOTFS/etc/os-release" ] && [ ! -f "$SCENARIO_ROOTFS/usr/lib/os-release" ]; then
    echo "BES Installer Live" > "$SCENARIO_ROOTFS/etc/os-release"
fi

# r[impl installer.no-reboot]: install a trap script that records if anything
# calls reboot inside the container.
REBOOT_SENTINEL="/tmp/bes-reboot-called"
for reboot_path in /usr/local/sbin/reboot /usr/sbin/reboot /sbin/reboot; do
    mkdir -p "$SCENARIO_ROOTFS/$(dirname "$reboot_path")"
    cat > "$SCENARIO_ROOTFS/$reboot_path" << 'TRAP'
#!/bin/sh
touch /tmp/bes-reboot-called
TRAP
    chmod +x "$SCENARIO_ROOTFS/$reboot_path"
done

# ============================================================
# Phase 4: Run installer inside systemd-nspawn
# ============================================================
echo "==> Running installer in systemd-nspawn container..."

# r[impl installer.container.isolation+2] (layer 1): only the loop device is
# bound into the container.
NSPAWN_BINDS=(
    "--bind=$LOOP_DEV"
)
NSPAWN_BINDS+=("--bind-ro=$IMAGES_DIR:/run/live/medium/images")
NSPAWN_BINDS+=("--bind-ro=$CONFIG_TOML:/run/besconf/bes-install.toml")
NSPAWN_BINDS+=("--bind-ro=$DEVICES_JSON:/tmp/devices.json")
NSPAWN_BINDS+=("--bind=/dev")

INSTALLER_LOG="$WORK_DIR/installer.log"
INSTALLER_OUTPUT="$WORK_DIR/installer-output.txt"

echo "    Running installer (disk-encryption=$DISK_ENCRYPTION, target=$LOOP_DEV)..."
echo ""

# r[impl installer.container.isolation+2] (layer 2): --fake-devices bypasses
# lsblk discovery so the installer sees only the loop device.
# r[impl installer.container.isolation+2] (layer 3): --private-network prevents
# any network side-effects from the container. This also serves as the
# enforcement mechanism for r[verify iso.offline]: a successful install with
# no network proves the ISO is fully self-contained.
set +e
systemd-nspawn \
    "${NSPAWN_COMMON_OPTS[@]}" \
    --directory="$SCENARIO_ROOTFS" \
    "${NSPAWN_BINDS[@]}" \
    /usr/local/bin/bes-installer \
        --fake-devices /tmp/devices.json \
        --config /run/besconf/bes-install.toml \
        --log /tmp/installer.log \
        --no-reboot \
    2>&1 | tee "$INSTALLER_OUTPUT"
INSTALLER_RC=${PIPESTATUS[0]}
set -e

if [ -f "$SCENARIO_ROOTFS/tmp/installer.log" ]; then
    cp "$SCENARIO_ROOTFS/tmp/installer.log" "$INSTALLER_LOG"
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

# r[verify iso.offline]: the installer completed successfully inside a
# container with --private-network, proving no network access was needed.
echo "    Installer exited successfully."

# r[verify installer.no-reboot]
if [ -f "$SCENARIO_ROOTFS/$REBOOT_SENTINEL" ]; then
    echo "    FAIL: reboot was called despite --no-reboot"
    exit 1
else
    echo "    PASS: reboot was not called (--no-reboot honored)"
fi

# ============================================================
# Phase 5: Verify cleanup
# ============================================================
# r[verify installer.firstboot.unmount]
echo "==> Verifying installer unmounted all filesystems..."

STALE_MOUNTS=$(grep "${LOOP_DEV}" /proc/mounts 2>/dev/null || true)
if [ -n "$STALE_MOUNTS" ]; then
    echo "    Stale mounts found:"
    printf '      %s\n' "$STALE_MOUNTS"
    echo "    FAIL: installer did not clean up mounts"
    exit 1
fi
echo "    PASS: no stale mounts from ${LOOP_DEV}"

# r[verify installer.write.luks-before-write]
if [ "$VARIANT" = "metal" ]; then
    if [ -e /dev/mapper/bes-target-root ]; then
        echo "    FAIL: LUKS volume bes-target-root still open"
        exit 1
    fi
    echo "    PASS: LUKS volume bes-target-root is closed"
fi

# ============================================================
# Phase 6: Verify the written disk
# ============================================================
echo "==> Verifying written disk..."

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

partprobe "$LOOP_DEV"
sleep 1

# --- Partition structure ---
check "partition 1 (EFI) exists" test -b "${LOOP_DEV}p1"
check "partition 2 (xboot) exists" test -b "${LOOP_DEV}p2"
check "partition 3 (root) exists" test -b "${LOOP_DEV}p3"

SFDISK_LABELS="$WORK_DIR/sfdisk-labels.txt"
sfdisk --json "$LOOP_DEV" 2>/dev/null > "$SFDISK_LABELS" || echo '{}' > "$SFDISK_LABELS"
check "EFI partition label present" grep -qi '"name"[[:space:]]*:[[:space:]]*"efi"' "$SFDISK_LABELS"
check "xboot partition label present" grep -qi '"name"[[:space:]]*:[[:space:]]*"xboot"' "$SFDISK_LABELS"
check "root partition label present" grep -qi '"name"[[:space:]]*:[[:space:]]*"root"' "$SFDISK_LABELS"

# r[verify installer.mode.auto.progress]
check "non-interactive write summary printed" grep -q "write complete:.*MiB in.*MiB/s" "$INSTALLER_OUTPUT"

# r[verify installer.write.partitions+2]
# r[verify installer.write.expand-root]
ROOT_PART_SIZE=$(lsblk --bytes --noheadings --output SIZE "${LOOP_DEV}p3" 2>/dev/null | tr -d '[:space:]')
ROOT_PART_SIZE="${ROOT_PART_SIZE:-0}"
IMAGE_RAW_SIZE=5368709120
echo "    Root partition size: $ROOT_PART_SIZE bytes (image was $IMAGE_RAW_SIZE bytes)"
check "root partition expanded beyond image size" test "$ROOT_PART_SIZE" -gt "$IMAGE_RAW_SIZE"

# --- Mount and verify first-boot configuration ---
VERIFY_MOUNT="$WORK_DIR/verify-mount"
mkdir -p "$VERIFY_MOUNT"

ROOT_PART="${LOOP_DEV}p3"

# r[verify installer.write.luks-before-write]
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

        # --- Hostname ---
        # r[verify installer.firstboot.hostname]
        if [ -n "$SET_HOSTNAME_FROM_DHCP" ]; then
            # DHCP hostname: /etc/hostname must be empty
            if [ -f "$VERIFY_MOUNT/etc/hostname" ]; then
                HOSTNAME_SIZE="$(stat -c%s "$VERIFY_MOUNT/etc/hostname")"
                check "hostname file is empty (DHCP mode)" \
                    test "$HOSTNAME_SIZE" = "0"
            else
                check "hostname file exists" false
            fi

            if [ -f "$VERIFY_MOUNT/etc/hosts" ]; then
                if grep -q "127.0.1.1" "$VERIFY_MOUNT/etc/hosts"; then
                    check "no 127.0.1.1 line in /etc/hosts (DHCP mode)" false
                else
                    check "no 127.0.1.1 line in /etc/hosts (DHCP mode)" true
                fi
            fi
        elif [ -n "$SET_HOSTNAME_TEMPLATE" ]; then
            # Template hostname: /etc/hostname must be non-empty and match pattern
            if [ -f "$VERIFY_MOUNT/etc/hostname" ]; then
                ACTUAL_HOSTNAME="$(tr -d '[:space:]' < "$VERIFY_MOUNT/etc/hostname")"
                check "hostname is non-empty (template mode)" \
                    test -n "$ACTUAL_HOSTNAME"
                # Extract the pattern from the template for basic validation.
                # For "test-{hex:6}" we check ^test-[0-9a-f]{6}$
                TEMPLATE_REGEX="${SET_HOSTNAME_TEMPLATE_REGEX:-}"
                if [ -n "$TEMPLATE_REGEX" ]; then
                    if echo "$ACTUAL_HOSTNAME" | grep -qE "$TEMPLATE_REGEX"; then
                        check "hostname matches template pattern '$TEMPLATE_REGEX'" true
                    else
                        check "hostname '$ACTUAL_HOSTNAME' matches template pattern '$TEMPLATE_REGEX'" false
                    fi
                fi
            else
                check "hostname file exists" false
            fi
        elif [ -n "$SET_HOSTNAME" ]; then
            if [ -f "$VERIFY_MOUNT/etc/hostname" ]; then
                ACTUAL_HOSTNAME="$(tr -d '[:space:]' < "$VERIFY_MOUNT/etc/hostname")"
                check "hostname is '$SET_HOSTNAME'" \
                    test "$ACTUAL_HOSTNAME" = "$SET_HOSTNAME"
            else
                check "hostname file exists" false
            fi

            if [ -f "$VERIFY_MOUNT/etc/hosts" ]; then
                check "/etc/hosts contains 127.0.1.1 entry for hostname" \
                    grep -q "127.0.1.1.*$SET_HOSTNAME" "$VERIFY_MOUNT/etc/hosts"
            else
                check "/etc/hosts file exists" false
            fi
        else
            echo "    (hostname not configured — skipping hostname checks)"
        fi

        # --- Tailscale authkey ---
        # r[verify installer.firstboot.tailscale-authkey]
        if [ -n "$SET_TAILSCALE" ]; then
            TS_KEY_FILE="$VERIFY_MOUNT/etc/bes/tailscale-authkey"
            if [ -f "$TS_KEY_FILE" ]; then
                check "tailscale-authkey contains configured key" \
                    grep -q "$SET_TAILSCALE" "$TS_KEY_FILE"

                TS_PERMS=$(stat -c '%a' "$TS_KEY_FILE")
                check "tailscale-authkey permissions are 600" test "$TS_PERMS" = "600"
            else
                check "tailscale-authkey file exists" false
            fi
        else
            TS_KEY_FILE="$VERIFY_MOUNT/etc/bes/tailscale-authkey"
            check "tailscale-authkey file absent when not configured" test ! -f "$TS_KEY_FILE"
        fi

        # --- SSH authorized keys ---
        # r[verify installer.firstboot.ssh-keys]
        if [ -n "$SET_SSH_KEYS" ]; then
            AK_FILE="$VERIFY_MOUNT/home/ubuntu/.ssh/authorized_keys"
            if [ -f "$AK_FILE" ]; then
                # Extract a unique substring from the key for matching
                KEY_FRAGMENT="$(echo "$SET_SSH_KEYS" | awk '{print $2}' | head -c 20)"
                check "authorized_keys contains test key" \
                    grep -q "$KEY_FRAGMENT" "$AK_FILE"

                AK_PERMS=$(stat -c '%a' "$AK_FILE")
                check "authorized_keys permissions are 600" test "$AK_PERMS" = "600"

                SSH_DIR_PERMS=$(stat -c '%a' "$VERIFY_MOUNT/home/ubuntu/.ssh")
                check ".ssh directory permissions are 700" test "$SSH_DIR_PERMS" = "700"
            else
                check "authorized_keys file exists" false
            fi
        else
            echo "    (ssh-keys not configured — skipping SSH checks)"
        fi

        # --- Password ---
        # r[verify installer.firstboot.password]
        if [ -n "$SET_PASSWORD" ] || [ -n "$SET_PASSWORD_HASH" ]; then
            SHADOW_FILE="$VERIFY_MOUNT/etc/shadow"
            if [ -f "$SHADOW_FILE" ]; then
                UBUNTU_SHADOW="$(grep '^ubuntu:' "$SHADOW_FILE")"
                if [ -n "$UBUNTU_SHADOW" ]; then
                    SHADOW_HASH="$(echo "$UBUNTU_SHADOW" | cut -d: -f2)"
                    check "ubuntu shadow entry has SHA-512 hash" \
                        test "${SHADOW_HASH#\$6\$}" != "$SHADOW_HASH"

                    SHADOW_LASTCHANGED="$(echo "$UBUNTU_SHADOW" | cut -d: -f3)"
                    check "ubuntu password expiry cleared (lastchanged > 0)" \
                        test "$SHADOW_LASTCHANGED" -gt 0

                    if [ -n "$SET_PASSWORD_HASH" ]; then
                        check "ubuntu shadow hash matches provided hash" \
                            test "$SHADOW_HASH" = "$SET_PASSWORD_HASH"
                    fi
                else
                    check "ubuntu user found in shadow" false
                fi
            else
                check "shadow file exists" false
            fi
        else
            echo "    (password not configured — skipping password checks)"
        fi

        # --- Timezone ---
        # r[verify installer.firstboot.timezone]
        if [ -n "$SET_TIMEZONE" ]; then
            LOCALTIME_LINK="$VERIFY_MOUNT/etc/localtime"
            if [ -L "$LOCALTIME_LINK" ]; then
                LOCALTIME_TARGET="$(readlink "$LOCALTIME_LINK")"
                check "localtime symlink points to $SET_TIMEZONE" \
                    test "$LOCALTIME_TARGET" = "/usr/share/zoneinfo/$SET_TIMEZONE"
            else
                check "localtime is a symlink" false
            fi

            TIMEZONE_FILE="$VERIFY_MOUNT/etc/timezone"
            if [ -f "$TIMEZONE_FILE" ]; then
                ACTUAL_TZ="$(tr -d '[:space:]' < "$TIMEZONE_FILE")"
                check "timezone file contains '$SET_TIMEZONE'" \
                    test "$ACTUAL_TZ" = "$SET_TIMEZONE"
            else
                check "timezone file exists" false
            fi
        else
            # Even without explicit timezone, installer should set UTC
            LOCALTIME_LINK="$VERIFY_MOUNT/etc/localtime"
            if [ -L "$LOCALTIME_LINK" ]; then
                LOCALTIME_TARGET="$(readlink "$LOCALTIME_LINK")"
                check "localtime symlink points to UTC (default)" \
                    test "$LOCALTIME_TARGET" = "/usr/share/zoneinfo/UTC"
            fi

            TIMEZONE_FILE="$VERIFY_MOUNT/etc/timezone"
            if [ -f "$TIMEZONE_FILE" ]; then
                ACTUAL_TZ="$(tr -d '[:space:]' < "$TIMEZONE_FILE")"
                check "timezone file contains 'UTC' (default)" \
                    test "$ACTUAL_TZ" = "UTC"
            fi
        fi

        # --- /etc/fstab references ---
        # r[verify installer.write.fstab-fixup]
        if [ "$VARIANT" = "metal" ]; then
            FSTAB="$VERIFY_MOUNT/etc/fstab"
            if [ -f "$FSTAB" ]; then
                check "fstab root entry uses /dev/mapper/root" \
                    grep -q '^/dev/mapper/root[[:space:]]' "$FSTAB"
                check "fstab has no by-partlabel/root references" \
                    test "$(grep -c 'by-partlabel/root' "$FSTAB")" -eq 0
            else
                check "fstab exists" false
            fi
        else
            FSTAB="$VERIFY_MOUNT/etc/fstab"
            if [ -f "$FSTAB" ]; then
                check "fstab root entry uses by-partlabel/root (cloud)" \
                    grep -q 'by-partlabel/root' "$FSTAB"
            else
                check "fstab exists" false
            fi
        fi

        # --- /etc/bes/image-variant ---
        # r[verify installer.write.variant-fixup]
        VARIANT_FILE="$VERIFY_MOUNT/etc/bes/image-variant"
        if [ -f "$VARIANT_FILE" ]; then
            ACTUAL_VARIANT="$(tr -d '[:space:]' < "$VARIANT_FILE")"
            check "image-variant is '$VARIANT'" \
                test "$ACTUAL_VARIANT" = "$VARIANT"
        else
            check "image-variant file exists" false
        fi

        # --- Install log ---
        # r[verify installer.firstboot.copy-install-log]
        INSTALL_LOG="$VERIFY_MOUNT/var/log/bes-installer.log"
        if [ "$SET_COPY_INSTALL_LOG" = "false" ]; then
            check "install log absent when copy-install-log=false" \
                test ! -f "$INSTALL_LOG"
        else
            if [ -f "$INSTALL_LOG" ]; then
                LOG_SIZE="$(stat -c%s "$INSTALL_LOG")"
                check "install log exists and is non-empty" \
                    test "$LOG_SIZE" -gt 0
            else
                check "install log exists" false
            fi
        fi

        # --- Encryption setup verification (metal only) ---
        # r[verify installer.encryption.overview]
        if [ "$VARIANT" = "metal" ]; then
            ROTATED_MARKER="$VERIFY_MOUNT/etc/luks/rotated"
            check "LUKS master key rotation marker exists" test -f "$ROTATED_MARKER"

            CRYPTTAB="$VERIFY_MOUNT/etc/crypttab"
            check "crypttab exists" test -f "$CRYPTTAB"
            if [ -f "$CRYPTTAB" ]; then
                check "crypttab references root partition" \
                    grep -q 'root' "$CRYPTTAB"
            fi
        fi

        # --- Filesystem UUID / grub.cfg consistency ---
        # r[verify installer.write.randomize-uuids]
        # r[verify installer.write.rebuild-boot-config]
        # Mount /boot so we can read grub.cfg
        BOOT_MNT="$WORK_DIR/verify-boot"
        mkdir -p "$BOOT_MNT"
        XBOOT_PART="${LOOP_DEV}p2"
        set +e
        mount -o ro "$XBOOT_PART" "$BOOT_MNT" 2>/dev/null
        BOOT_MOUNT_RC=$?
        set -e
        if [ $BOOT_MOUNT_RC -eq 0 ]; then
            GRUB_CFG="$BOOT_MNT/grub/grub.cfg"
            if [ ! -f "$GRUB_CFG" ]; then
                GRUB_CFG="$BOOT_MNT/efi/EFI/ubuntu/grub.cfg"
            fi
            if [ -f "$GRUB_CFG" ]; then
                # Get the actual BTRFS UUID of the root device
                ACTUAL_ROOT_UUID="$(blkid -o value -s UUID "$BTRFS_DEV" 2>/dev/null || true)"
                if [ -n "$ACTUAL_ROOT_UUID" ]; then
                    check "grub.cfg references actual root UUID ($ACTUAL_ROOT_UUID)" \
                        grep -q "$ACTUAL_ROOT_UUID" "$GRUB_CFG"
                else
                    echo "    (could not read root UUID — skipping grub.cfg UUID check)"
                fi
            else
                echo "    (grub.cfg not found — skipping UUID consistency check)"
            fi
            umount "$BOOT_MNT"
        else
            echo "    (could not mount xboot — skipping grub.cfg checks)"
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
# Results
# ============================================================
echo ""
echo "--- Scenario '$SCENARIO_NAME': $PASS passed, $FAIL failed ---"

if [ ${#ERRORS[@]} -gt 0 ]; then
    echo ""
    echo "  Failures:"
    for e in "${ERRORS[@]}"; do
        echo "    - $e"
    done
fi

if [ -f "$INSTALLER_LOG" ]; then
    echo "  Installer log: $INSTALLER_LOG"
fi

echo ""

if [ "$FAIL" -eq 0 ]; then
    exit 0
else
    exit 1
fi
