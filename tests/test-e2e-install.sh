#!/bin/bash
#
# End-to-end install test: boot the live ISO in QEMU with a blank virtual disk
# and an injected bes-install.toml for fully automatic installation, then boot
# the installed system and run smoke checks.
#
# Usage: test-e2e-install.sh <iso> <variant> <arch>
#   variant: metal | cloud
#   arch:    amd64 | arm64
#
# Requires: qemu-system-{x86_64,aarch64}, qemu-img, genisoimage, xorriso, UEFI firmware
set -euo pipefail

ISO="${1:?Usage: $0 <iso> <variant> <arch>}"
VARIANT="${2:?Usage: $0 <iso> <variant> <arch>}"
ARCH="${3:?Usage: $0 <iso> <variant> <arch>}"

INSTALL_TIMEOUT="${INSTALL_TIMEOUT:-600}"
BOOT_TIMEOUT="${BOOT_TIMEOUT:-300}"
TARGET_DISK_SIZE="${TARGET_DISK_SIZE:-16G}"
QEMU_MEMORY="${QEMU_MEMORY:-4096}"
QEMU_CORES="${QEMU_CORES:-2}"

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
    amd64)
        QEMU_CMD="qemu-system-x86_64"
        QEMU_ACCEL="-enable-kvm"
        if [ ! -e /dev/kvm ]; then
            echo "WARNING: /dev/kvm not available, falling back to software emulation (slow)"
            QEMU_ACCEL=""
        fi
        FW_CODE=""
        FW_VARS=""
        for f in /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2/x64/OVMF_CODE.fd /usr/share/edk2-ovmf/x64/OVMF_CODE.4m.fd; do
            if [ -f "$f" ]; then FW_CODE="$f"; break; fi
        done
        for f in /usr/share/OVMF/OVMF_VARS.fd /usr/share/edk2/x64/OVMF_VARS.fd /usr/share/edk2-ovmf/x64/OVMF_VARS.4m.fd; do
            if [ -f "$f" ]; then FW_VARS="$f"; break; fi
        done
        ;;
    arm64)
        QEMU_CMD="qemu-system-aarch64"
        QEMU_ACCEL="-machine virt"
        if [ -e /dev/kvm ] && [ "$(uname -m)" = "aarch64" ]; then
            QEMU_ACCEL="-enable-kvm -machine virt"
        else
            QEMU_ACCEL="-machine virt -cpu cortex-a57"
        fi
        FW_CODE=""
        FW_VARS=""
        for f in /usr/share/AAVMF/AAVMF_CODE.fd /usr/share/edk2/aarch64/QEMU_CODE.fd /usr/share/qemu-efi-aarch64/QEMU_EFI.fd; do
            if [ -f "$f" ]; then FW_CODE="$f"; break; fi
        done
        for f in /usr/share/AAVMF/AAVMF_VARS.fd /usr/share/edk2/aarch64/QEMU_VARS.fd; do
            if [ -f "$f" ]; then FW_VARS="$f"; break; fi
        done
        ;;
    *)
        echo "ERROR: arch must be amd64 or arm64 (got: $ARCH)"
        exit 1
        ;;
esac

if [ -z "$FW_CODE" ] || [ -z "$FW_VARS" ]; then
    echo "ERROR: UEFI firmware not found for $ARCH"
    echo "  Install: apt-get install ovmf (amd64) or qemu-efi-aarch64 (arm64)"
    exit 1
fi

for cmd in "$QEMU_CMD" qemu-img genisoimage xorriso; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "ERROR: $cmd not found"
        exit 1
    fi
done

WORK_DIR="$(mktemp -d -t bes-e2e-XXXXXX)"

cleanup() {
    local exit_code=$?
    set +e
    rm -rf "$WORK_DIR"
    exit "$exit_code"
}
trap cleanup EXIT

echo "=============================="
echo "BES E2E Install Test"
echo "=============================="
echo "ISO:       $ISO"
echo "Variant:   $VARIANT"
echo "Arch:      $ARCH"
echo "QEMU:      $QEMU_CMD"
echo "Firmware:  $FW_CODE"
echo "Work dir:  $WORK_DIR"
echo "=============================="
echo ""

# ============================================================
# Phase 1: Inject bes-install.toml into ISO's BESCONF partition
# ============================================================
echo "==> Injecting bes-install.toml into ISO's BESCONF partition..."

MODIFIED_ISO="$WORK_DIR/installer.iso"
cp "$ISO" "$MODIFIED_ISO"

cat > "$WORK_DIR/bes-install.toml" << EOF
auto = true
disk-encryption = "$([ "$VARIANT" = "metal" ] && echo "keyfile" || echo "none")"
disk = "largest"
hostname = "e2e-test-$VARIANT"
EOF

echo "    Config:"
sed 's/^/      /' "$WORK_DIR/bes-install.toml"

# The BESCONF partition is an appended partition in the hybrid ISO's GPT.
# xorriso inserts gap partitions, so the number is not predictable.
# Find it by PARTLABEL instead.
BESCONF_LINE=""
while IFS= read -r line; do
    if echo "$line" | grep -q "Appended3"; then
        BESCONF_LINE="$line"
        break
    fi
done < <(sgdisk --print "$MODIFIED_ISO" 2>/dev/null)

if [ -z "$BESCONF_LINE" ]; then
    echo "ERROR: could not find BESCONF (Appended3) partition in ISO GPT"
    echo "sgdisk output:"
    sgdisk --print "$MODIFIED_ISO" 2>/dev/null || true
    exit 1
fi

BESCONF_START_SECTOR="$(echo "$BESCONF_LINE" | awk '{print $2}')"
BESCONF_END_SECTOR="$(echo "$BESCONF_LINE" | awk '{print $3}')"
BESCONF_SECTOR_COUNT=$(( BESCONF_END_SECTOR - BESCONF_START_SECTOR + 1 ))

echo "    BESCONF partition: start=${BESCONF_START_SECTOR} sectors=${BESCONF_SECTOR_COUNT}"

# Extract the BESCONF FAT image, mount it, inject the config, put it back
BESCONF_IMG="$WORK_DIR/besconf.img"
dd if="$MODIFIED_ISO" of="$BESCONF_IMG" bs=512 \
    skip="$BESCONF_START_SECTOR" count="$BESCONF_SECTOR_COUNT" status=none

mkdir -p "$WORK_DIR/besconf-mount"
sudo mount -o loop "$BESCONF_IMG" "$WORK_DIR/besconf-mount"
sudo cp "$WORK_DIR/bes-install.toml" "$WORK_DIR/besconf-mount/bes-install.toml"
sudo umount "$WORK_DIR/besconf-mount"

# Write the modified FAT image back into the ISO at the same offset
dd if="$BESCONF_IMG" of="$MODIFIED_ISO" bs=512 \
    seek="$BESCONF_START_SECTOR" conv=notrunc status=none

echo "    Injected config into BESCONF partition."

# ============================================================
# Phase 2: Create blank target disk
# ============================================================
echo "==> Creating blank target disk ($TARGET_DISK_SIZE)..."
TARGET_DISK="$WORK_DIR/target-disk.qcow2"
qemu-img create -f qcow2 "$TARGET_DISK" "$TARGET_DISK_SIZE" >/dev/null

# Writable copy of firmware vars
FW_VARS_COPY="$WORK_DIR/fw_vars.fd"
cp "$FW_VARS" "$FW_VARS_COPY"

# ============================================================
# Phase 3: Boot ISO and run automatic install
# ============================================================
INSTALL_LOG="$WORK_DIR/install-serial.log"

echo "==> Phase 1: Booting ISO for automatic install (timeout: ${INSTALL_TIMEOUT}s)..."
echo "    Serial log: $INSTALL_LOG"

# shellcheck disable=SC2086
timeout "$INSTALL_TIMEOUT" \
    "$QEMU_CMD" $QEMU_ACCEL \
    -m "$QEMU_MEMORY" \
    -smp "$QEMU_CORES" \
    -nographic \
    -serial mon:stdio \
    -drive if=pflash,format=raw,readonly=on,file="$FW_CODE" \
    -drive if=pflash,format=raw,file="$FW_VARS_COPY" \
    -drive file="$MODIFIED_ISO",format=raw,if=virtio,readonly=on \
    -drive file="$TARGET_DISK",format=qcow2,if=virtio \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0 \
    -no-reboot \
    2>&1 | tee "$INSTALL_LOG" || true

echo ""

# Check if the installer completed (look for reboot or completion markers)
if grep -q "installation complete" "$INSTALL_LOG" || grep -q "rebooting" "$INSTALL_LOG"; then
    echo "    Install phase completed successfully."
else
    echo "    WARNING: Could not confirm install completion from serial log."
    echo "    Last 20 lines:"
    tail -20 "$INSTALL_LOG"
    echo ""
    echo "    Proceeding to boot test anyway..."
fi

# ============================================================
# Phase 4: Create cloud-init NoCloud ISO for smoke test
# ============================================================
echo "==> Creating cloud-init smoke test ISO..."

CI_DIR="$WORK_DIR/cidata"
mkdir -p "$CI_DIR"

cat > "$CI_DIR/meta-data" << META
instance-id: e2e-test
local-hostname: e2e-test-$VARIANT
META

cat > "$CI_DIR/user-data" << 'CLOUDINIT'
#cloud-config
runcmd:
  - |
    #!/bin/bash
    exec > /dev/ttyS0 2>&1

    PASS=0
    FAIL=0
    ERRORS=()

    check() {
      local desc="$1"; shift
      if "$@" >/dev/null 2>&1; then
        echo "PASS: $desc"
        PASS=$((PASS + 1))
      else
        echo "FAIL: $desc"
        ERRORS+=("$desc")
        FAIL=$((FAIL + 1))
      fi
    }

    echo "=== BES E2E Smoke Test ==="
    echo ""

    check "systemd reached multi-user.target" systemctl is-active multi-user.target
    FAILED_UNITS=$(systemctl --failed --no-legend --no-pager | wc -l)
    check "no failed systemd units" test "$FAILED_UNITS" -eq 0

    check "sshd is active" systemctl is-active ssh
    check "ufw is active" systemctl is-active ufw
    check "tailscaled is active" systemctl is-active tailscaled
    check "snapper-timeline.timer is active" systemctl is-active snapper-timeline.timer

    check "root is btrfs" test "$(stat -f -c%T /)" = "btrfs"
    check "compression active in /proc/mounts" grep -q 'compress=' /proc/mounts

    VARIANT=$(cat /etc/bes/image-variant 2>/dev/null || echo "unknown")
    echo "Variant: $VARIANT"

    if [ "$VARIANT" = "metal" ]; then
      check "LUKS volume is active" test -e /dev/mapper/root
    fi

    check "ubuntu user exists" id ubuntu
    check "machine-id is non-empty" test -s /etc/machine-id
    check "/boot is mounted" mountpoint -q /boot
    check "/boot/efi is mounted" mountpoint -q /boot/efi

    ACTUAL_HOSTNAME="$(hostname)"
    echo "Hostname: $ACTUAL_HOSTNAME"
    check "hostname was configured" echo "$ACTUAL_HOSTNAME" | grep -q "e2e-test"

    echo ""
    echo "RESULTS: $PASS passed, $FAIL failed"

    if [ $FAIL -eq 0 ]; then
      echo "TEST_SUCCESS"
    else
      echo "TEST_FAILURE"
      for e in "${ERRORS[@]}"; do
        echo "  - $e"
      done
    fi

    sleep 2
    poweroff
CLOUDINIT

genisoimage -output "$WORK_DIR/cidata.iso" \
    -volid cidata -joliet -rock \
    "$CI_DIR/meta-data" "$CI_DIR/user-data" >/dev/null 2>&1

# ============================================================
# Phase 5: Boot installed system with smoke test
# ============================================================
BOOT_LOG="$WORK_DIR/boot-serial.log"

echo "==> Phase 2: Booting installed system (timeout: ${BOOT_TIMEOUT}s)..."
echo "    Serial log: $BOOT_LOG"

# Reset firmware vars for a clean boot
cp "$FW_VARS" "$FW_VARS_COPY"

# shellcheck disable=SC2086
timeout "$BOOT_TIMEOUT" \
    "$QEMU_CMD" $QEMU_ACCEL \
    -m "$QEMU_MEMORY" \
    -smp "$QEMU_CORES" \
    -nographic \
    -serial mon:stdio \
    -drive if=pflash,format=raw,readonly=on,file="$FW_CODE" \
    -drive if=pflash,format=raw,file="$FW_VARS_COPY" \
    -drive file="$TARGET_DISK",format=qcow2,if=virtio \
    -drive file="$WORK_DIR/cidata.iso",format=raw,if=virtio \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0 \
    -no-reboot \
    2>&1 | tee "$BOOT_LOG" || true

echo ""

# ============================================================
# Phase 6: Evaluate results
# ============================================================
echo "=== E2E Test Results ($VARIANT / $ARCH) ==="
echo ""

if grep -q "TEST_SUCCESS" "$BOOT_LOG"; then
    echo "E2E install test PASSED"
    echo ""
    grep "^PASS:" "$BOOT_LOG" | sed 's/^/  /'
    echo ""
    exit 0
elif grep -q "TEST_FAILURE" "$BOOT_LOG"; then
    echo "E2E install test FAILED"
    echo ""
    grep -E "^(PASS|FAIL):" "$BOOT_LOG" | sed 's/^/  /'
    echo ""
    exit 1
else
    echo "E2E install test DID NOT COMPLETE (timeout or crash)"
    echo ""
    echo "Last 50 lines of boot serial log:"
    tail -50 "$BOOT_LOG"
    echo ""
    echo "Last 50 lines of install serial log:"
    tail -50 "$INSTALL_LOG"
    exit 1
fi
