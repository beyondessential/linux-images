#!/bin/bash
# Verify the structure of a built live installer ISO without booting it.
# This runs in CI without KVM — it inspects the ISO via xorriso, mounts
# the squashfs, and checks the appended BESCONF partition.
#
# Usage: test-iso-structure.sh <iso-file> <arch> [installer-bin]
#   arch: amd64 | arm64
#   installer-bin: optional path to host bes-installer binary (for --check-paths)
set -euo pipefail

ISO="${1:?Usage: $0 <iso-file> <arch> [installer-bin]}"
ARCH="${2:?Usage: $0 <iso-file> <arch> [installer-bin]}"
INSTALLER_BIN="${3:-}"

PASS=0
FAIL=0
ERRORS=()

pass() {
    local desc="$1"
    echo "  PASS: $desc"
    PASS=$((PASS + 1))
}

fail() {
    local desc="$1"
    echo "  FAIL: $desc"
    ERRORS+=("$desc")
    FAIL=$((FAIL + 1))
}

check() {
    local desc="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        pass "$desc"
    else
        fail "$desc"
    fi
}

# --- Pre-flight ---
if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root (need losetup/mount)"
    exit 1
fi

if [ ! -f "$ISO" ]; then
    echo "ERROR: ISO not found: $ISO"
    exit 1
fi

for cmd in xorriso sgdisk blkid file losetup jq; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "ERROR: required command not found: $cmd"
        exit 1
    fi
done

echo "=============================="
echo "ISO Structure Verification"
echo "=============================="
echo "ISO:  $ISO"
echo "Arch: $ARCH"
echo "=============================="
echo ""

# ============================================================
# State tracking for cleanup
# ============================================================
ISO_MNT=""
SQFS_MNT=""
BESCONF_MNT=""
LOOP_DEVICE=""
DD_IMG=""
DD_LOOP=""
ISO_MOUNTED=0
SQFS_MOUNTED=0
BESCONF_MOUNTED=0

cleanup() {
    set +e
    [ "$BESCONF_MOUNTED" -eq 1 ] && umount "$BESCONF_MNT" 2>/dev/null
    [ "$SQFS_MOUNTED" -eq 1 ] && umount "$SQFS_MNT" 2>/dev/null
    [ "$ISO_MOUNTED" -eq 1 ] && umount "$ISO_MNT" 2>/dev/null
    [ -n "$LOOP_DEVICE" ] && losetup -d "$LOOP_DEVICE" 2>/dev/null
    [ -n "$DD_LOOP" ] && losetup -d "$DD_LOOP" 2>/dev/null
    [ -n "$DD_IMG" ] && rm -f "$DD_IMG"
    [ -n "$ISO_MNT" ] && rmdir "$ISO_MNT" 2>/dev/null
    [ -n "$SQFS_MNT" ] && rmdir "$SQFS_MNT" 2>/dev/null
    [ -n "$BESCONF_MNT" ] && rmdir "$BESCONF_MNT" 2>/dev/null
}
trap cleanup EXIT

ISO_MNT="$(mktemp -d -t iso-mnt-XXXXXX)"
SQFS_MNT="$(mktemp -d -t sqfs-mnt-XXXXXX)"
BESCONF_MNT="$(mktemp -d -t besconf-mnt-XXXXXX)"

# ============================================================
# 1. ISO9660 format checks
# ============================================================
echo "--- ISO9660 Format ---"

# r[verify iso.format]
FILE_TYPE="$(file -b "$ISO")"
if echo "$FILE_TYPE" | grep -qi "ISO 9660"; then
    pass "file is ISO9660"
else
    fail "file is ISO9660 (got: $FILE_TYPE)"
fi

# r[verify iso.format]
VOLID="$(xorriso -indev "$ISO" -pvd_info 2>&1 | grep "Volume Id" | sed 's/.*: *//' | tr -d '[:space:]')"
if [ -z "$VOLID" ]; then
    VOLID="$(xorriso -indev "$ISO" -pvd_info 2>&1 | grep "Volume id" | sed 's/.*: *//' | tr -d '[:space:]')"
fi
if [ "$VOLID" = "BES_INSTALLER" ]; then
    pass "volume ID is BES_INSTALLER"
else
    fail "volume ID is BES_INSTALLER (got: '$VOLID')"
fi

# ============================================================
# 2. Hybrid GPT checks
# ============================================================
echo ""
echo "--- Hybrid GPT ---"

# r[verify iso.hybrid]
PTTYPE="$(blkid -o value -s PTTYPE "$ISO" 2>/dev/null || true)"
if [ "$PTTYPE" = "gpt" ] || sgdisk -p "$ISO" 2>/dev/null | grep -q "^Number"; then
    pass "ISO contains a GPT"
else
    fail "ISO contains a GPT"
fi

# r[verify iso.hybrid]
ESP_FOUND=0
while IFS= read -r line; do
    if echo "$line" | grep -qi "EF00\|C12A7328-F81F-11D2-BA4B-00A0C93EC93B\|EFI [Ss]ystem"; then
        ESP_FOUND=1
        break
    fi
done < <(sgdisk -p "$ISO" 2>/dev/null || true)
if [ "$ESP_FOUND" -eq 1 ]; then
    pass "GPT contains an EFI System Partition"
else
    fail "GPT contains an EFI System Partition"
fi

# ============================================================
# 3. El Torito boot catalog
# ============================================================
echo ""
echo "--- El Torito Boot ---"

# r[verify iso.boot.uefi]
ELTORITO_OUTPUT="$(xorriso -indev "$ISO" -report_el_torito plain 2>&1 || true)"
if echo "$ELTORITO_OUTPUT" | grep -qi "El Torito\|boot catalog\|efi\|platform.*EFI"; then
    pass "El Torito boot catalog is present"
else
    # Try alternate detection: check for boot/efi.img in catalog
    if echo "$ELTORITO_OUTPUT" | grep -qi "efi.img\|boot.*image"; then
        pass "El Torito boot catalog is present"
    else
        fail "El Torito boot catalog is present"
    fi
fi

# ============================================================
# 4. Mount ISO and check contents
# ============================================================
echo ""
echo "--- ISO Contents ---"

mount -o loop,ro "$ISO" "$ISO_MNT"
ISO_MOUNTED=1

# r[verify iso.live-boot]
check "/live/filesystem.squashfs exists" test -f "$ISO_MNT/live/filesystem.squashfs"

# Verify squashfs is actually a squashfs
if [ -f "$ISO_MNT/live/filesystem.squashfs" ]; then
    SQFS_TYPE="$(file -b "$ISO_MNT/live/filesystem.squashfs")"
    if echo "$SQFS_TYPE" | grep -qi "squashfs"; then
        pass "/live/filesystem.squashfs is valid squashfs"
    else
        fail "/live/filesystem.squashfs is valid squashfs (got: $SQFS_TYPE)"
    fi
fi

# r[verify iso.base+2]
check "/live/vmlinuz exists" test -f "$ISO_MNT/live/vmlinuz"
check "/live/initrd.img exists" test -f "$ISO_MNT/live/initrd.img"

# r[verify iso.boot.uefi]
if [ "$ARCH" = "amd64" ]; then
    check "EFI/BOOT/BOOTX64.EFI exists" test -f "$ISO_MNT/EFI/BOOT/BOOTX64.EFI"
else
    check "EFI/BOOT/BOOTAA64.EFI exists" test -f "$ISO_MNT/EFI/BOOT/BOOTAA64.EFI"
fi

# r[verify iso.boot.uefi]
check "/boot/grub/grub.cfg exists" test -f "$ISO_MNT/boot/grub/grub.cfg"

# Verify grub.cfg contains boot=live
if [ -f "$ISO_MNT/boot/grub/grub.cfg" ]; then
    check "grub.cfg contains boot=live" grep -q "boot=live" "$ISO_MNT/boot/grub/grub.cfg"
fi

# r[verify iso.contents+2]
check "partitions.json exists" test -f "$ISO_MNT/images/partitions.json"

# Verify partitions.json is valid JSON with expected structure
if [ -f "$ISO_MNT/images/partitions.json" ]; then
    if jq empty "$ISO_MNT/images/partitions.json" 2>/dev/null; then
        pass "partitions.json is valid JSON"
    else
        fail "partitions.json is valid JSON"
    fi

    MANIFEST_ARCH="$(jq -r '.arch' "$ISO_MNT/images/partitions.json" 2>/dev/null)"
    if [ "$MANIFEST_ARCH" = "$ARCH" ]; then
        pass "partitions.json arch matches expected ($ARCH)"
    else
        fail "partitions.json arch matches expected ($ARCH, got: $MANIFEST_ARCH)"
    fi

    PART_COUNT="$(jq '.partitions | length' "$ISO_MNT/images/partitions.json" 2>/dev/null)"
    if [ "$PART_COUNT" -eq 3 ]; then
        pass "partitions.json has 3 partitions"
    else
        fail "partitions.json has 3 partitions (got: $PART_COUNT)"
    fi

    # Verify each partition entry has the required fields
    for field in label type_uuid size_mib image; do
        MISSING_FIELD="$(jq -r ".partitions[] | select(.${field} == null) | .label // \"unknown\"" "$ISO_MNT/images/partitions.json" 2>/dev/null)"
        if [ -z "$MISSING_FIELD" ]; then
            pass "all partitions have '$field' field"
        else
            fail "all partitions have '$field' field (missing in: $MISSING_FIELD)"
        fi
    done

    # Verify expected partition labels
    for label in efi xboot root; do
        FOUND_LABEL="$(jq -r ".partitions[] | select(.label == \"$label\") | .label" "$ISO_MNT/images/partitions.json" 2>/dev/null)"
        if [ "$FOUND_LABEL" = "$label" ]; then
            pass "partitions.json contains '$label' partition"
        else
            fail "partitions.json contains '$label' partition"
        fi
    done
fi

# r[verify iso.contents+2]
# Verify partition image files and their .size sidecars
for name in efi xboot root; do
    check "${name}.img.zst exists" test -f "$ISO_MNT/images/${name}.img.zst"

    # Verify it's actually zstd-compressed
    if [ -f "$ISO_MNT/images/${name}.img.zst" ]; then
        IMG_TYPE="$(file -b "$ISO_MNT/images/${name}.img.zst")"
        if echo "$IMG_TYPE" | grep -qi "zstandard"; then
            pass "${name}.img.zst is valid zstd"
        else
            fail "${name}.img.zst is valid zstd (got: $IMG_TYPE)"
        fi
    fi

    # r[verify installer.write.disk-size-check+2]
    if [ -f "$ISO_MNT/images/${name}.img.size" ]; then
        pass ".size sidecar exists for ${name}.img.zst"
        SIZE_VALUE="$(cat "$ISO_MNT/images/${name}.img.size")"
        if [ "$SIZE_VALUE" -gt 0 ] 2>/dev/null; then
            pass "${name}.img.size contains a positive number ($SIZE_VALUE)"
        else
            fail "${name}.img.size contains a positive number (got: $SIZE_VALUE)"
        fi
    else
        fail ".size sidecar exists for ${name}.img.zst"
    fi
done

# Verify no whole-disk .raw.zst images are present (old format)
OLD_IMAGE_COUNT="$(find "$ISO_MNT/images" -maxdepth 1 -name '*.raw.zst' 2>/dev/null | wc -l)"
if [ "$OLD_IMAGE_COUNT" -eq 0 ]; then
    pass "no old whole-disk .raw.zst images in /images/"
else
    fail "no old whole-disk .raw.zst images in /images/ (found $OLD_IMAGE_COUNT)"
fi

# r[verify iso.boot.uefi]
check "/boot/efi.img exists" test -f "$ISO_MNT/boot/efi.img"

# ============================================================
# 5. Mount squashfs and check rootfs contents
# ============================================================
echo ""
echo "--- Squashfs Rootfs ---"

if [ -f "$ISO_MNT/live/filesystem.squashfs" ]; then
    mount -o loop,ro "$ISO_MNT/live/filesystem.squashfs" "$SQFS_MNT"
    SQFS_MOUNTED=1

    # r[verify iso.contents+2]
    check "bes-installer binary exists" test -x "$SQFS_MNT/usr/local/bin/bes-installer"

    # r[verify installer.hardcoded-paths]
    if [ -n "$INSTALLER_BIN" ] && [ -x "$INSTALLER_BIN" ]; then
        CHECK_OUTPUT="$("$INSTALLER_BIN" --check-paths "$SQFS_MNT" 2>&1)"
        CHECK_RC=$?
        if [ "$CHECK_RC" -eq 0 ]; then
            pass "bes-installer --check-paths against squashfs"
        else
            fail "bes-installer --check-paths against squashfs"
            echo "$CHECK_OUTPUT" | sed 's/^/    /'
        fi
    else
        if [ -n "$INSTALLER_BIN" ]; then
            fail "bes-installer --check-paths (binary not found: $INSTALLER_BIN)"
        else
            echo "  SKIP: bes-installer --check-paths (no installer-bin argument provided)"
        fi
    fi

    # r[verify iso.boot.autostart+3]
    check "bes-installer-wrapper exists" test -x "$SQFS_MNT/usr/local/bin/bes-installer-wrapper"
    check "bes-installer.service exists" test -f "$SQFS_MNT/etc/systemd/system/bes-installer.service"
    check "bes-chvt.service exists" test -f "$SQFS_MNT/etc/systemd/system/bes-chvt.service"
    check "chvt binary exists (kbd package)" test -x "$SQFS_MNT/usr/bin/chvt"
    check "reboot binary exists (systemd-sysv)" test -x "$SQFS_MNT/sbin/reboot" -o -x "$SQFS_MNT/usr/sbin/reboot"

    # Check that the services are enabled (symlinked into .wants)
    if find "$SQFS_MNT/etc/systemd/system" -name "bes-installer.service" -type l 2>/dev/null | grep -q .; then
        pass "bes-installer.service is enabled"
    else
        fail "bes-installer.service is enabled"
    fi
    if find "$SQFS_MNT/etc/systemd/system" -name "bes-chvt.service" -type l 2>/dev/null | grep -q .; then
        pass "bes-chvt.service is enabled"
    else
        fail "bes-chvt.service is enabled"
    fi

    # r[verify iso.live-boot]
    if [ -x "$SQFS_MNT/usr/bin/dpkg-query" ]; then
        mount -t proc proc "$SQFS_MNT/proc" 2>/dev/null || true
        # shellcheck disable=SC2016 # ${Status} is a dpkg format string
        if chroot "$SQFS_MNT" dpkg-query -W -f='${Status}\n' live-boot 2>/dev/null | grep -q "install ok installed"; then
            pass "live-boot package is installed"
        else
            fail "live-boot package is installed"
        fi
        # shellcheck disable=SC2016
        if chroot "$SQFS_MNT" dpkg-query -W -f='${Status}\n' live-boot-initramfs-tools 2>/dev/null | grep -q "install ok installed"; then
            pass "live-boot-initramfs-tools package is installed"
        else
            fail "live-boot-initramfs-tools package is installed"
        fi
        umount "$SQFS_MNT/proc" 2>/dev/null || true
    else
        fail "dpkg-query not found in squashfs — cannot verify packages"
    fi

    # r[verify iso.minimal+2]
    if [ -x "$SQFS_MNT/sbin/cryptsetup" ] || [ -x "$SQFS_MNT/usr/sbin/cryptsetup" ]; then
        pass "cryptsetup exists in rootfs"
    else
        fail "cryptsetup exists in rootfs"
    fi
    check "zstd exists in rootfs" test -x "$SQFS_MNT/usr/bin/zstd"
    if [ -x "$SQFS_MNT/sbin/sgdisk" ] || [ -x "$SQFS_MNT/usr/sbin/sgdisk" ]; then
        pass "sgdisk exists in rootfs"
    else
        fail "sgdisk exists in rootfs"
    fi

    # r[verify iso.network-tools+3]
    check "curl exists in rootfs" test -x "$SQFS_MNT/usr/bin/curl"
    if [ -x "$SQFS_MNT/usr/bin/tailscale" ] || [ -x "$SQFS_MNT/usr/sbin/tailscale" ]; then
        pass "tailscale exists in rootfs"
    else
        fail "tailscale exists in rootfs"
    fi
    check "ip command exists (iproute2)" test -x "$SQFS_MNT/usr/sbin/ip" -o -x "$SQFS_MNT/sbin/ip" -o -x "$SQFS_MNT/usr/bin/ip" -o -x "$SQFS_MNT/bin/ip"
    check "ping command exists (iputils-ping)" test -x "$SQFS_MNT/usr/bin/ping" -o -x "$SQFS_MNT/bin/ping"

    # r[verify iso.blacklist-drm]
    check "GPU blacklist exists" test -f "$SQFS_MNT/etc/modprobe.d/blacklist-gpu.conf"
    check "blacklist blocks vmwgfx" grep -q 'install vmwgfx /bin/false' "$SQFS_MNT/etc/modprobe.d/blacklist-gpu.conf"

    # r[verify iso.network-config+2]
    check "netplan DHCP config exists" test -f "$SQFS_MNT/etc/netplan/01-all-en-dhcp.yaml"
    check "netplan config matches en* interfaces" grep -q 'name:.*en\*' "$SQFS_MNT/etc/netplan/01-all-en-dhcp.yaml"
    check "netplan config enables dhcp4" grep -q 'dhcp4:.*true' "$SQFS_MNT/etc/netplan/01-all-en-dhcp.yaml"
    check "resolv.conf is symlink to resolved stub" test -L "$SQFS_MNT/etc/resolv.conf"

    # r[verify iso.config-partition]
    check "run-besconf.mount exists" test -f "$SQFS_MNT/etc/systemd/system/run-besconf.mount"
    check "run-besconf.automount exists" test -f "$SQFS_MNT/etc/systemd/system/run-besconf.automount"
    if find "$SQFS_MNT/etc/systemd/system" -name "run-besconf.automount" -type l 2>/dev/null | grep -q .; then
        pass "run-besconf.automount is enabled"
    else
        fail "run-besconf.automount is enabled"
    fi

    # Verify build info
    check "/etc/bes-build-info exists" test -f "$SQFS_MNT/etc/bes-build-info"
    if [ -f "$SQFS_MNT/etc/bes-build-info" ]; then
        check "bes-build-info contains BUILD_DATE" grep -q "^BUILD_DATE=" "$SQFS_MNT/etc/bes-build-info"
        check "bes-build-info contains ARCH" grep -q "^ARCH=" "$SQFS_MNT/etc/bes-build-info"

        # r[verify iso.per-arch]
        BUILT_ARCH="$(grep "^ARCH=" "$SQFS_MNT/etc/bes-build-info" | cut -d= -f2)"
        if [ "$BUILT_ARCH" = "$ARCH" ]; then
            pass "bes-build-info ARCH matches expected ($ARCH)"
        else
            fail "bes-build-info ARCH matches expected ($ARCH, got: $BUILT_ARCH)"
        fi
    fi

    # r[verify iso.boot.autostart+3]
    # getty on tty2 should be masked so it doesn't compete with the installer
    if [ -L "$SQFS_MNT/etc/systemd/system/getty@tty2.service" ]; then
        MASK_TARGET="$(readlink "$SQFS_MNT/etc/systemd/system/getty@tty2.service")"
        if [ "$MASK_TARGET" = "/dev/null" ]; then
            pass "getty@tty2.service is masked"
        else
            fail "getty@tty2.service is masked (points to: $MASK_TARGET)"
        fi
    else
        fail "getty@tty2.service is masked"
    fi

    umount "$SQFS_MNT" 2>/dev/null
    SQFS_MOUNTED=0
else
    fail "cannot check squashfs contents — file missing"
fi

# ============================================================
# 6. BESCONF partition check
# ============================================================
echo ""
echo "--- BESCONF Partition ---"

# r[verify iso.config-partition]
# Set up a loop device with partition scanning to find the appended BESCONF partition.
LOOP_DEVICE="$(losetup -f --show -P "$ISO")"
partprobe "$LOOP_DEVICE" 2>/dev/null || true
udevadm settle 2>/dev/null || true
sleep 1

BESCONF_PART=""
for part in "${LOOP_DEVICE}p"*; do
    [ -b "$part" ] || continue
    LABEL="$(blkid -o value -s LABEL "$part" 2>/dev/null || true)"
    if [ "$LABEL" = "BESCONF" ]; then
        BESCONF_PART="$part"
        break
    fi
done

if [ -n "$BESCONF_PART" ]; then
    pass "BESCONF partition found ($BESCONF_PART)"

    BESCONF_FSTYPE="$(blkid -o value -s TYPE "$BESCONF_PART" 2>/dev/null || true)"
    if [ "$BESCONF_FSTYPE" = "vfat" ]; then
        pass "BESCONF is FAT filesystem"
    else
        fail "BESCONF is FAT filesystem (got: $BESCONF_FSTYPE)"
    fi

    mount -o ro "$BESCONF_PART" "$BESCONF_MNT"
    BESCONF_MOUNTED=1

    check "bes-install.toml template exists on BESCONF" test -f "$BESCONF_MNT/bes-install.toml"

    umount "$BESCONF_MNT"
    BESCONF_MOUNTED=0
else
    fail "BESCONF partition found"
fi

losetup -d "$LOOP_DEVICE"
LOOP_DEVICE=""

# ============================================================
# 7. USB dd write verification
# ============================================================
echo ""
echo "--- USB dd Write ---"

# r[verify iso.usb]: write the ISO to a sparse file via dd (simulating a
# USB write), then verify the result has a valid GPT with an EFI System
# Partition. This proves the hybrid layout survives a raw block copy.
DD_IMG="$(mktemp -t bes-dd-usb-XXXXXX.img)"
dd if="$ISO" of="$DD_IMG" bs=4M status=none
check "dd write succeeds" test -f "$DD_IMG"

DD_LOOP="$(losetup -f --show -P "$DD_IMG")"
partprobe "$DD_LOOP" 2>/dev/null || true
udevadm settle 2>/dev/null || true
sleep 1

# The dd output must have a valid GPT
if sgdisk -p "$DD_LOOP" 2>/dev/null | grep -q "^Number"; then
    pass "dd output has valid GPT"
else
    fail "dd output has valid GPT"
fi

# The dd output must contain an EFI System Partition
DD_ESP_FOUND=0
while IFS= read -r line; do
    if echo "$line" | grep -qi "EF00\|C12A7328-F81F-11D2-BA4B-00A0C93EC93B\|EFI [Ss]ystem"; then
        DD_ESP_FOUND=1
        break
    fi
done < <(sgdisk -p "$DD_LOOP" 2>/dev/null || true)
if [ "$DD_ESP_FOUND" -eq 1 ]; then
    pass "dd output GPT contains EFI System Partition"
else
    fail "dd output GPT contains EFI System Partition"
fi

# The BESCONF partition must also survive the dd
DD_BESCONF_FOUND=0
for part in "${DD_LOOP}p"*; do
    [ -b "$part" ] || continue
    LABEL="$(blkid -o value -s LABEL "$part" 2>/dev/null || true)"
    if [ "$LABEL" = "BESCONF" ]; then
        DD_BESCONF_FOUND=1
        break
    fi
done
if [ "$DD_BESCONF_FOUND" -eq 1 ]; then
    pass "dd output contains BESCONF partition"
else
    fail "dd output contains BESCONF partition"
fi

losetup -d "$DD_LOOP"
DD_LOOP=""
rm -f "$DD_IMG"
DD_IMG=""

# ============================================================
# Results
# ============================================================
echo ""
echo "=============================="
echo "RESULTS: $PASS passed, $FAIL failed"
echo "=============================="

if [ $FAIL -gt 0 ]; then
    echo ""
    echo "Failures:"
    for e in "${ERRORS[@]}"; do
        echo "  - $e"
    done
    echo ""
    exit 1
fi
