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
if [ -n "$INSTALLER_BIN" ] && [ "${INSTALLER_BIN#/}" = "$INSTALLER_BIN" ]; then
    INSTALLER_BIN="$PWD/$INSTALLER_BIN"
fi

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

for cmd in xorriso sgdisk blkid file losetup jq veritysetup; do
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
IMAGES_MNT=""
LOOP_DEVICE=""
DD_IMG=""
DD_LOOP=""
ISO_MOUNTED=0
SQFS_MOUNTED=0
BESCONF_MOUNTED=0
IMAGES_MOUNTED=0

cleanup() {
    set +e
    [ "$IMAGES_MOUNTED" -eq 1 ] && umount "$IMAGES_MNT" 2>/dev/null
    [ "$BESCONF_MOUNTED" -eq 1 ] && umount "$BESCONF_MNT" 2>/dev/null
    [ "$SQFS_MOUNTED" -eq 1 ] && umount "$SQFS_MNT" 2>/dev/null
    [ "$ISO_MOUNTED" -eq 1 ] && umount "$ISO_MNT" 2>/dev/null
    [ -n "$LOOP_DEVICE" ] && losetup -d "$LOOP_DEVICE" 2>/dev/null
    [ -n "$DD_LOOP" ] && losetup -d "$DD_LOOP" 2>/dev/null
    [ -n "$DD_IMG" ] && rm -f "$DD_IMG"
    [ -n "$ISO_MNT" ] && rmdir "$ISO_MNT" 2>/dev/null
    [ -n "$SQFS_MNT" ] && rmdir "$SQFS_MNT" 2>/dev/null
    [ -n "$BESCONF_MNT" ] && rmdir "$BESCONF_MNT" 2>/dev/null
    [ -n "$IMAGES_MNT" ] && rmdir "$IMAGES_MNT" 2>/dev/null
}
trap cleanup EXIT

ISO_MNT="$(mktemp -d -t iso-mnt-XXXXXX)"
SQFS_MNT="$(mktemp -d -t sqfs-mnt-XXXXXX)"
BESCONF_MNT="$(mktemp -d -t besconf-mnt-XXXXXX)"
IMAGES_MNT="$(mktemp -d -t images-mnt-XXXXXX)"

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

    # r[verify iso.verity.squashfs+2]
    check "grub.cfg contains live.verity.roothash" grep -q "live.verity.roothash=" "$ISO_MNT/boot/grub/grub.cfg"

    # r[verify iso.verity.images+4]
    check "grub.cfg contains images.verity.roothash" grep -q "images.verity.roothash=" "$ISO_MNT/boot/grub/grub.cfg"
fi

# r[verify iso.contents+3]
# Partition images are no longer in the ISO9660 filesystem.
# They live in a dedicated GPT partition (the images squashfs with verity).
# Verify the old /images/ directory is NOT present in the ISO.
if [ -d "$ISO_MNT/images" ]; then
    fail "no /images/ directory in ISO9660 (images moved to images GPT partition)"
else
    pass "no /images/ directory in ISO9660 (images moved to images GPT partition)"
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

    # r[verify iso.contents+3]
    check "bes-installer binary exists" test -x "$SQFS_MNT/usr/local/bin/bes-installer"

    # r[verify installer.hardcoded-paths]
    if [ -n "$INSTALLER_BIN" ] && [ -x "$INSTALLER_BIN" ]; then
        CHECK_OUTPUT="$("$INSTALLER_BIN" --check-paths "$SQFS_MNT" 2>&1)" && CHECK_RC=0 || CHECK_RC=$?
        if [ "$CHECK_RC" -eq 0 ]; then
            pass "bes-installer --check-paths (ISO binaries) against squashfs"
        else
            fail "bes-installer --check-paths (ISO binaries) against squashfs"
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

    # r[verify iso.minimal+3]
    if [ -x "$SQFS_MNT/sbin/cryptsetup" ] || [ -x "$SQFS_MNT/usr/sbin/cryptsetup" ]; then
        pass "cryptsetup exists in rootfs"
    else
        fail "cryptsetup exists in rootfs"
    fi
    if [ -x "$SQFS_MNT/sbin/veritysetup" ] || [ -x "$SQFS_MNT/usr/sbin/veritysetup" ]; then
        pass "veritysetup exists in rootfs"
    else
        fail "veritysetup exists in rootfs"
    fi
    check "zstd exists in rootfs" test -x "$SQFS_MNT/usr/bin/zstd"
    if [ -x "$SQFS_MNT/sbin/sgdisk" ] || [ -x "$SQFS_MNT/usr/sbin/sgdisk" ]; then
        pass "sgdisk exists in rootfs"
    else
        fail "sgdisk exists in rootfs"
    fi

    # r[verify iso.verity.initramfs-hook]
    check "verity initramfs hook exists" test -f "$SQFS_MNT/usr/share/initramfs-tools/hooks/verity"
    check "verity premount script exists" test -f "$SQFS_MNT/usr/share/initramfs-tools/scripts/live-premount/verity"

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

    # r[verify iso.config-partition+2]
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

# r[verify iso.config-partition+2]
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

# ============================================================
# 6b. Images partition check (by type UUID)
# ============================================================
echo ""
echo "--- Images Partition ---"

# r[verify iso.images-partition+2]
# r[verify iso.verity.images+4]
# r[verify iso.verity.layout+3]
# Find the images partition by GPT type UUID (Linux filesystem).
# We cannot use a hardcoded partition number because xorriso may renumber
# partitions relative to the -append_partition arguments.
GPT_TYPE_LINUX_FILESYSTEM="0FC63DAF-8483-4772-8E79-3D69D8477DE4"
IMAGES_PART=""
while IFS= read -r line; do
    # sgdisk -p output: "   N  start  end  size  code  name"
    PARTNUM="$(echo "$line" | awk '{print $1}')"
    CODE="$(echo "$line" | awk '{print $6}')"
    if [ "$CODE" = "8300" ]; then
        CANDIDATE="${LOOP_DEVICE}p${PARTNUM}"
        if [ -b "$CANDIDATE" ]; then
            IMAGES_PART="$CANDIDATE"
            break
        fi
    fi
done < <(sgdisk -p "$LOOP_DEVICE" 2>/dev/null | grep '^ *[0-9]')

if [ -n "$IMAGES_PART" ]; then
    pass "images partition found by type UUID ($IMAGES_PART)"

    # Verify the verity trailer: last 8 bytes are a LE u64 hash size
    IMAGES_TOTAL_SIZE="$(blockdev --getsize64 "$IMAGES_PART")"
    if [ "$IMAGES_TOTAL_SIZE" -gt 8 ]; then
        TRAILER_BYTES="$(dd if="$IMAGES_PART" bs=1 skip=$((IMAGES_TOTAL_SIZE - 8)) count=8 2>/dev/null | od -A n -t u1 | tr -s ' ')"
        HASH_SIZE=0
        SHIFT=0
        for b in $TRAILER_BYTES; do
            HASH_SIZE=$((HASH_SIZE + (b << SHIFT)))
            SHIFT=$((SHIFT + 8))
        done

        if [ "$HASH_SIZE" -gt 0 ] && [ "$HASH_SIZE" -lt "$IMAGES_TOTAL_SIZE" ]; then
            pass "images partition has valid verity trailer (hash_size=$HASH_SIZE)"
        else
            fail "images partition has valid verity trailer (hash_size=$HASH_SIZE, total=$IMAGES_TOTAL_SIZE)"
        fi
    else
        fail "images partition is large enough for verity trailer ($IMAGES_TOTAL_SIZE bytes)"
    fi

    # r[verify iso.verity.images+4]
    # Extract the root hash from grub.cfg and verify the images partition
    IMAGES_ROOTHASH=""
    if [ -f "$ISO_MNT/boot/grub/grub.cfg" ]; then
        IMAGES_ROOTHASH="$(grep -o 'images\.verity\.roothash=[^ ]*' "$ISO_MNT/boot/grub/grub.cfg" | head -1 | cut -d= -f2)"
    fi
    if [ -n "$IMAGES_ROOTHASH" ]; then
        HASH_OFFSET=$((IMAGES_TOTAL_SIZE - 8 - HASH_SIZE))
        # r[verify iso.verity.build-deps]
        if veritysetup verify "$IMAGES_PART" "$IMAGES_PART" "$IMAGES_ROOTHASH" --hash-offset="$HASH_OFFSET" 2>/dev/null; then
            pass "images partition passes veritysetup verify"
        else
            fail "images partition passes veritysetup verify"
        fi

        # Open verity, mount, and check contents
        if veritysetup open "$IMAGES_PART" test-besimages "$IMAGES_PART" "$IMAGES_ROOTHASH" --hash-offset="$HASH_OFFSET" 2>/dev/null; then
            mount -t squashfs -o ro /dev/mapper/test-besimages "$IMAGES_MNT" 2>/dev/null && IMAGES_MOUNTED=1

            if [ "$IMAGES_MOUNTED" -eq 1 ]; then
                pass "images squashfs mounts via verity"

                # r[verify iso.contents+3]
                check "partitions.json in images squashfs" test -f "$IMAGES_MNT/partitions.json"

                if [ -f "$IMAGES_MNT/partitions.json" ]; then
                    if jq empty "$IMAGES_MNT/partitions.json" 2>/dev/null; then
                        pass "partitions.json is valid JSON"
                    else
                        fail "partitions.json is valid JSON"
                    fi

                    MANIFEST_ARCH="$(jq -r '.arch' "$IMAGES_MNT/partitions.json" 2>/dev/null)"
                    if [ "$MANIFEST_ARCH" = "$ARCH" ]; then
                        pass "partitions.json arch matches expected ($ARCH)"
                    else
                        fail "partitions.json arch matches expected ($ARCH, got: $MANIFEST_ARCH)"
                    fi

                    PART_COUNT="$(jq '.partitions | length' "$IMAGES_MNT/partitions.json" 2>/dev/null)"
                    if [ "$PART_COUNT" -eq 3 ]; then
                        pass "partitions.json has 3 partitions"
                    else
                        fail "partitions.json has 3 partitions (got: $PART_COUNT)"
                    fi

                    for field in label type_uuid size_mib image; do
                        MISSING_FIELD="$(jq -r ".partitions[] | select(.${field} == null) | .label // \"unknown\"" "$IMAGES_MNT/partitions.json" 2>/dev/null)"
                        if [ -z "$MISSING_FIELD" ]; then
                            pass "all partitions have '$field' field"
                        else
                            fail "all partitions have '$field' field (missing in: $MISSING_FIELD)"
                        fi
                    done

                    for label in efi xboot root; do
                        FOUND_LABEL="$(jq -r ".partitions[] | select(.label == \"$label\") | .label" "$IMAGES_MNT/partitions.json" 2>/dev/null)"
                        if [ "$FOUND_LABEL" = "$label" ]; then
                            pass "partitions.json contains '$label' partition"
                        else
                            fail "partitions.json contains '$label' partition"
                        fi
                    done

                    # r[verify iso.contents+3]
                    # Verify raw image files (no .zst, no .size sidecars)
                    for name in efi xboot root; do
                        check "${name}.img exists in images squashfs" test -f "$IMAGES_MNT/${name}.img"

                        # r[verify installer.write.disk-size-check+3]
                        if [ -f "$IMAGES_MNT/${name}.img" ]; then
                            IMG_SIZE="$(stat --format='%s' "$IMAGES_MNT/${name}.img")"
                            if [ "$IMG_SIZE" -gt 0 ] 2>/dev/null; then
                                pass "${name}.img has positive size ($IMG_SIZE bytes)"
                            else
                                fail "${name}.img has positive size (got: $IMG_SIZE)"
                            fi
                        fi

                        # No .zst or .size sidecars should exist
                        if [ -f "$IMAGES_MNT/${name}.img.zst" ]; then
                            fail "no ${name}.img.zst in images squashfs (old format)"
                        else
                            pass "no ${name}.img.zst in images squashfs (old format)"
                        fi
                        if [ -f "$IMAGES_MNT/${name}.img.size" ]; then
                            fail "no ${name}.img.size sidecar in images squashfs"
                        else
                            pass "no ${name}.img.size sidecar in images squashfs"
                        fi
                    done
                fi

                umount "$IMAGES_MNT"
                IMAGES_MOUNTED=0
            else
                fail "images squashfs mounts via verity"
            fi

            veritysetup close test-besimages 2>/dev/null
        else
            fail "images partition verity open succeeded"
        fi
    else
        fail "images.verity.roothash found in grub.cfg"
    fi
else
    fail "images partition found by type UUID (Linux filesystem)"
fi

# r[verify iso.verity.squashfs+2]
# Verify the squashfs rootfs verity trailer
echo ""
echo "--- Squashfs Verity ---"

if [ -f "$ISO_MNT/live/filesystem.squashfs" ]; then
    SQFS_TOTAL_SIZE="$(stat --format='%s' "$ISO_MNT/live/filesystem.squashfs")"
    if [ "$SQFS_TOTAL_SIZE" -gt 8 ]; then
        SQFS_TRAILER_BYTES="$(dd if="$ISO_MNT/live/filesystem.squashfs" bs=1 skip=$((SQFS_TOTAL_SIZE - 8)) count=8 2>/dev/null | od -A n -t u1 | tr -s ' ')"
        SQFS_HASH_SIZE=0
        SQFS_SHIFT=0
        for b in $SQFS_TRAILER_BYTES; do
            SQFS_HASH_SIZE=$((SQFS_HASH_SIZE + (b << SQFS_SHIFT)))
            SQFS_SHIFT=$((SQFS_SHIFT + 8))
        done

        if [ "$SQFS_HASH_SIZE" -gt 0 ] && [ "$SQFS_HASH_SIZE" -lt "$SQFS_TOTAL_SIZE" ]; then
            pass "squashfs rootfs has valid verity trailer (hash_size=$SQFS_HASH_SIZE)"
        else
            fail "squashfs rootfs has valid verity trailer (hash_size=$SQFS_HASH_SIZE, total=$SQFS_TOTAL_SIZE)"
        fi

        LIVE_ROOTHASH=""
        if [ -f "$ISO_MNT/boot/grub/grub.cfg" ]; then
            LIVE_ROOTHASH="$(grep -o 'live\.verity\.roothash=[^ ]*' "$ISO_MNT/boot/grub/grub.cfg" | head -1 | cut -d= -f2)"
        fi
        if [ -n "$LIVE_ROOTHASH" ]; then
            SQFS_HASH_OFFSET=$((SQFS_TOTAL_SIZE - 8 - SQFS_HASH_SIZE))
            # Use a loop device for veritysetup (it needs a block device)
            SQFS_LOOP="$(losetup -f --show -r "$ISO_MNT/live/filesystem.squashfs")"
            if veritysetup verify "$SQFS_LOOP" "$SQFS_LOOP" "$LIVE_ROOTHASH" --hash-offset="$SQFS_HASH_OFFSET" 2>/dev/null; then
                pass "squashfs rootfs passes veritysetup verify"
            else
                fail "squashfs rootfs passes veritysetup verify"
            fi
            losetup -d "$SQFS_LOOP"
        else
            fail "live.verity.roothash found in grub.cfg for verification"
        fi
    else
        fail "squashfs rootfs is large enough for verity trailer ($SQFS_TOTAL_SIZE bytes)"
    fi
else
    fail "cannot check squashfs verity — file missing"
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

# r[verify iso.images-partition+2]
# Images partition must also survive the dd — find by type UUID, not number
DD_IMAGES_FOUND=0
while IFS= read -r line; do
    PARTNUM="$(echo "$line" | awk '{print $1}')"
    CODE="$(echo "$line" | awk '{print $6}')"
    if [ "$CODE" = "8300" ]; then
        CANDIDATE="${DD_LOOP}p${PARTNUM}"
        if [ -b "$CANDIDATE" ]; then
            DD_IMAGES_FOUND=1
            break
        fi
    fi
done < <(sgdisk -p "$DD_LOOP" 2>/dev/null | grep '^ *[0-9]')
if [ "$DD_IMAGES_FOUND" -eq 1 ]; then
    pass "dd output contains images partition (by type UUID)"
else
    fail "dd output contains images partition (by type UUID)"
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
