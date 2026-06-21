#!/bin/bash
# Loopback-mount a built image and verify its structure without booting.
# This runs in CI without KVM.
#
# Usage: test-image-structure.sh <image.img> <variant> <arch>
#   variant: metal | cloud | pi
#   arch:    amd64 | arm64
set -euo pipefail

IMAGE="${1:?Usage: $0 <image.img> <variant> <arch>}"
VARIANT="${2:?Usage: $0 <image.img> <variant> <arch>}"
ARCH="${3:?Usage: $0 <image.img> <variant> <arch>}"

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

check_not() {
    local desc="$1"
    shift
    if ! "$@" >/dev/null 2>&1; then
        pass "$desc"
    else
        fail "$desc"
    fi
}

# Compare two dotted version strings: returns 0 (true) if $1 >= $2
version_ge() {
    printf '%s\n%s\n' "$2" "$1" | sort -V -C
}

# Query a package version from the chroot and check it meets a minimum.
# Usage: check_pkg_version <package> <min_version> <tracey_tag>
check_pkg_version() {
    local pkg="$1" min="$2" tag="$3"
    # shellcheck disable=SC2016 # ${Version} is a dpkg format string
    local ver
    ver="$(chroot "$MNT" dpkg-query -W -f='${Version}\n' "$pkg" 2>/dev/null || true)"
    if [ -z "$ver" ]; then
        fail "$tag: $pkg is installed"
        return
    fi
    # Strip epoch (e.g. "2:5.3.1-1" -> "5.3.1-1") and debian revision
    local upstream
    upstream="${ver#*:}"       # remove epoch
    upstream="${upstream%%-*}" # remove debian revision
    if version_ge "$upstream" "$min"; then
        pass "$tag: $pkg version $ver >= $min"
    else
        fail "$tag: $pkg version $ver >= $min"
    fi
}

# --- Pre-flight ---
if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root (need losetup/mount)"
    exit 1
fi

if [ ! -f "$IMAGE" ]; then
    echo "ERROR: image not found: $IMAGE"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PACKAGES_FILE="$REPO_ROOT/image/packages.sh"

echo "=============================="
echo "Image Structure Verification"
echo "=============================="
echo "Image:   $IMAGE"
echo "Variant: $VARIANT"
echo "Arch:    $ARCH"
echo "=============================="
echo ""

# ============================================================
# State tracking for cleanup
# ============================================================
LOOP_DEVICE=""
MNT="/mnt/test-image-$$"
LUKS_NAME="test-root-$$"
ROOT_MOUNTED=0
PG_MOUNTED=0
BOOT_MOUNTED=0
EFI_MOUNTED=0

# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    set +e
    [ "$EFI_MOUNTED" -eq 1 ]  && umount "$MNT/boot/efi" 2>/dev/null
    [ "$EFI_MOUNTED" -eq 1 ]  && umount "$MNT/boot/firmware" 2>/dev/null
    [ "$BOOT_MOUNTED" -eq 1 ] && umount "$MNT/boot" 2>/dev/null
    [ "$PG_MOUNTED" -eq 1 ]   && umount "$MNT/var/lib/postgresql" 2>/dev/null
    [ "$ROOT_MOUNTED" -eq 1 ] && umount "$MNT" 2>/dev/null
    if [ "$VARIANT" = "metal" ] || [ "$VARIANT" = "pi" ]; then
        cryptsetup close "$LUKS_NAME" 2>/dev/null
    fi
    [ -n "$LOOP_DEVICE" ] && losetup -d "$LOOP_DEVICE" 2>/dev/null
    rmdir "$MNT" 2>/dev/null
}
trap cleanup EXIT

# ============================================================
# 1. Partition table checks
# ============================================================
echo "--- Partition Table ---"

LOOP_DEVICE="$(losetup -f --show -P "$IMAGE")"
partprobe "$LOOP_DEVICE" 2>/dev/null || true
udevadm settle 2>/dev/null || true
sleep 1

# r[verify image.partition.table]
PTTYPE="$(blkid -o value -s PTTYPE "$LOOP_DEVICE" 2>/dev/null || true)"
check "partition table is GPT" [ "$PTTYPE" = "gpt" ]

# r[verify image.partition.count]
PART_COUNT="$(lsblk -ln -o NAME "$LOOP_DEVICE" | grep -c "^$(basename "$LOOP_DEVICE")p")"
if [ "$PART_COUNT" -eq 3 ]; then
    pass "partition count is 3"
else
    fail "partition count is 3 (got $PART_COUNT)"
fi

# Helper to read partition info via sgdisk
get_part_label() { sgdisk -i "$1" "$LOOP_DEVICE" 2>/dev/null | grep "Partition name" | sed "s/.*'\(.*\)'/\1/"; }
get_part_type() { sgdisk -i "$1" "$LOOP_DEVICE" 2>/dev/null | grep "Partition GUID code" | awk '{print $4}'; }

# r[verify image.partition.efi] r[verify image.partition.pi-firmware]
EFI_LABEL="$(get_part_label 1)"
EFI_TYPE="$(get_part_type 1)"
if [ "$VARIANT" = "pi" ]; then
    check "partition 1 label is 'firmware'" [ "$EFI_LABEL" = "firmware" ]
else
    check "partition 1 label is 'efi'" [ "$EFI_LABEL" = "efi" ]
fi
check "partition 1 type is EFI System" [ "$EFI_TYPE" = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B" ]

# r[verify image.partition.xboot]
XBOOT_LABEL="$(get_part_label 2)"
XBOOT_TYPE="$(get_part_type 2)"
check "partition 2 label is 'xboot'" [ "$XBOOT_LABEL" = "xboot" ]
check "partition 2 type is Linux extended boot" [ "$XBOOT_TYPE" = "BC13C2FF-59E6-4262-A352-B275FD6F7172" ]

# r[verify image.partition.root]
ROOT_LABEL="$(get_part_label 3)"
ROOT_TYPE="$(get_part_type 3)"
check "partition 3 label is 'root'" [ "$ROOT_LABEL" = "root" ]

if [ "$ARCH" = "amd64" ]; then
    EXPECTED_ROOT_TYPE="4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709"
else
    EXPECTED_ROOT_TYPE="B921B045-1DF0-41C3-AF44-4C6F280D3FAE"
fi
check "partition 3 type UUID matches $ARCH" [ "$ROOT_TYPE" = "$EXPECTED_ROOT_TYPE" ]

EFI_PART="${LOOP_DEVICE}p1"
BOOT_PART="${LOOP_DEVICE}p2"
ROOT_PART="${LOOP_DEVICE}p3"

# ============================================================
# 2. Filesystem type checks
# ============================================================
echo ""
echo "--- Filesystems ---"

# r[verify image.partition.efi]
EFI_FSTYPE="$(blkid -o value -s TYPE "$EFI_PART" 2>/dev/null || true)"
check "EFI partition is vfat" [ "$EFI_FSTYPE" = "vfat" ]

# r[verify image.partition.xboot]
BOOT_FSTYPE="$(blkid -o value -s TYPE "$BOOT_PART" 2>/dev/null || true)"
check "boot partition is ext4" [ "$BOOT_FSTYPE" = "ext4" ]

BOOT_FSLABEL="$(blkid -o value -s LABEL "$BOOT_PART" 2>/dev/null || true)"
check "boot partition label is 'xboot'" [ "$BOOT_FSLABEL" = "xboot" ]

# r[verify image.luks.format]
if [ "$VARIANT" = "metal" ] || [ "$VARIANT" = "pi" ]; then
    ROOT_FSTYPE="$(blkid -o value -s TYPE "$ROOT_PART" 2>/dev/null || true)"
    check "root partition is crypto_LUKS ($VARIANT)" [ "$ROOT_FSTYPE" = "crypto_LUKS" ]

    LUKS_VERSION="$(cryptsetup luksDump "$ROOT_PART" 2>/dev/null | grep "^Version:" | awk '{print $2}')"
    check "LUKS version is 2" [ "$LUKS_VERSION" = "2" ]

    # Open LUKS with empty passphrase
    KEYFILE="$(mktemp)"
    truncate -s 0 "$KEYFILE"
    if cryptsetup open "$ROOT_PART" "$LUKS_NAME" --key-file "$KEYFILE" 2>/dev/null; then
        pass "LUKS opens with empty passphrase"
    else
        fail "LUKS opens with empty passphrase"
        rm -f "$KEYFILE"
        # Can't continue without root access
        echo ""
        echo "FATAL: Cannot open LUKS volume — skipping remaining checks"
        echo ""
        echo "RESULTS: $PASS passed, $FAIL failed"
        exit 1
    fi
    rm -f "$KEYFILE"
    BTRFS_DEV="/dev/mapper/$LUKS_NAME"
else
    ROOT_FSTYPE="$(blkid -o value -s TYPE "$ROOT_PART" 2>/dev/null || true)"
    check "root partition is btrfs (cloud)" [ "$ROOT_FSTYPE" = "btrfs" ]
    BTRFS_DEV="$ROOT_PART"
fi

# r[verify image.btrfs.format+2]
BTRFS_LABEL="$(blkid -o value -s LABEL "$BTRFS_DEV" 2>/dev/null || true)"
check "BTRFS label is 'ROOT'" [ "$BTRFS_LABEL" = "ROOT" ]

# ============================================================
# 3. BTRFS subvolume checks
# ============================================================
echo ""
echo "--- BTRFS Subvolumes ---"

mkdir -p "$MNT"

# Mount the raw BTRFS (no subvol) to check subvolumes
mount "$BTRFS_DEV" "$MNT" -o compress=zstd:6
ROOT_MOUNTED=1

# r[verify image.btrfs.subvolumes]
SUBVOLS="$(btrfs subvolume list "$MNT" 2>/dev/null | awk '{print $NF}')"
if echo "$SUBVOLS" | grep -qx "@"; then pass "subvolume '@' exists"; else fail "subvolume '@' exists"; fi
if echo "$SUBVOLS" | grep -qx "@postgres"; then pass "subvolume '@postgres' exists"; else fail "subvolume '@postgres' exists"; fi

# r[verify image.btrfs.quotas]
QUOTA_OUTPUT="$(btrfs qgroup show "$MNT" 2>/dev/null || true)"
if [ -n "$QUOTA_OUTPUT" ]; then
    pass "BTRFS quotas are enabled"
else
    fail "BTRFS quotas are enabled"
fi

# Unmount bare BTRFS
umount "$MNT"
ROOT_MOUNTED=0

# ============================================================
# 4. Mount the full filesystem tree and check contents
# ============================================================
echo ""
echo "--- File Checks ---"

# Mount @ subvolume as root
mount "$BTRFS_DEV" "$MNT" -o subvol=@,compress=zstd:6
ROOT_MOUNTED=1

mkdir -p "$MNT/var/lib/postgresql"
mount "$BTRFS_DEV" "$MNT/var/lib/postgresql" -o subvol=@postgres,compress=zstd:6
PG_MOUNTED=1

mkdir -p "$MNT/boot"
mount "$BOOT_PART" "$MNT/boot"
BOOT_MOUNTED=1

if [ "$VARIANT" = "pi" ]; then
    mkdir -p "$MNT/boot/firmware"
    mount "$EFI_PART" "$MNT/boot/firmware"
else
    mkdir -p "$MNT/boot/efi"
    mount "$EFI_PART" "$MNT/boot/efi"
fi
EFI_MOUNTED=1

# Detect the Ubuntu suite of the built image so suite-specific checks can
# be gated appropriately (e.g. the noble dracut hostonly workaround).
SUITE="$(. "$MNT/etc/os-release" 2>/dev/null; echo "${VERSION_CODENAME:-}")"
echo "Detected suite: ${SUITE:-<unknown>}"

# r[verify image.base.debootstrap]
check "/etc/fstab exists" test -f "$MNT/etc/fstab"

# r[verify image.variant.types+3]
check "/etc/bes/image-variant exists" test -f "$MNT/etc/bes/image-variant"

# r[verify image.tailscale.ts-up]
check "/usr/local/bin/ts-up exists" test -x "$MNT/usr/local/bin/ts-up"
# r[verify image.tailscale.firstboot-auth]
check "/usr/local/bin/bes-tailscale-firstboot-auth exists" test -x "$MNT/usr/local/bin/bes-tailscale-firstboot-auth"

# r[verify image.firstboot.script]
check "/usr/local/bin/bes-firstboot-script exists" test -x "$MNT/usr/local/bin/bes-firstboot-script"
check "/etc/systemd/system/bes-firstboot-script.service exists" test -f "$MNT/etc/systemd/system/bes-firstboot-script.service"
check "/etc/bes/firstboot-script default manifest is empty" test -f "$MNT/etc/bes/firstboot-script" -a ! -s "$MNT/etc/bes/firstboot-script"
check "/etc/bes/firstboot-script.done marker absent in pristine image" test ! -e "$MNT/etc/bes/firstboot-script.done"
check "bes-firstboot-script.service is gated on marker absence" \
    grep -qx 'ConditionPathExists=!/etc/bes/firstboot-script.done' "$MNT/etc/systemd/system/bes-firstboot-script.service"
if [ "$VARIANT" = "pi" ]; then
    check "/boot/firmware/firstboot-script default manifest is empty (pi)" \
        test -f "$MNT/boot/firmware/firstboot-script" -a ! -s "$MNT/boot/firmware/firstboot-script"
fi

# r[verify image.growth.service+3]
check "/usr/local/bin/grow-root-filesystem exists" test -x "$MNT/usr/local/bin/grow-root-filesystem"

# r[verify image.growth.service+3]
check "/etc/systemd/system/grow-root-filesystem.service exists" test -f "$MNT/etc/systemd/system/grow-root-filesystem.service"

# r[verify image.variant.types+3]
ACTUAL_VARIANT="$(cat "$MNT/etc/bes/image-variant" 2>/dev/null || true)"
check "image-variant contains '$VARIANT'" [ "$ACTUAL_VARIANT" = "$VARIANT" ]

# r[verify image.base.machine-id] r[verify image.cloud-init.no-machineid]
MACHINE_ID_SIZE="$(stat -c%s "$MNT/etc/machine-id" 2>/dev/null || echo "missing")"
check "/etc/machine-id is empty (size=0)" [ "$MACHINE_ID_SIZE" = "0" ]

# A populated machine-id in the initramfs gets committed to the empty rootfs
# /etc/machine-id at switch-root, producing duplicate IDs across every flashed
# install. Empty bytes or systemd's all-zeros UUID both register as
# uninitialized and trigger regeneration on first boot.
INITRD_MID="$(chroot "$MNT" bash -c 'lsinitrd -f etc/machine-id /boot/initrd.img-* 2>/dev/null' | tr -d '[:space:]' || true)"
case "$INITRD_MID" in
    "" | "00000000000000000000000000000000")
        pass "initramfs /etc/machine-id is uninitialized"
        ;;
    *)
        fail "initramfs /etc/machine-id is uninitialized (got '$INITRD_MID')"
        ;;
esac

# r[verify image.hostname.metal-dhcp+2] r[verify image.hostname.cloud-default+2]
if [ "$VARIANT" = "metal" ] || [ "$VARIANT" = "pi" ]; then
    HOSTNAME_SIZE="$(stat -c%s "$MNT/etc/hostname" 2>/dev/null || echo "missing")"
    check "/etc/hostname is empty for $VARIANT (size=0)" [ "$HOSTNAME_SIZE" = "0" ]
    check_not "/etc/hosts has no 127.0.1.1 line for $VARIANT" grep -q '127\.0\.1\.1' "$MNT/etc/hosts"
else
    HOSTNAME_CONTENT="$(tr -d '[:space:]' < "$MNT/etc/hostname" 2>/dev/null || echo "")"
    check "/etc/hostname contains 'ubuntu' for cloud" [ "$HOSTNAME_CONTENT" = "ubuntu" ]
fi

# r[verify image.base.resolver]
check "/etc/resolv.conf is a symlink" test -L "$MNT/etc/resolv.conf"
RESOLV_TARGET="$(readlink "$MNT/etc/resolv.conf" 2>/dev/null || true)"
check "/etc/resolv.conf points to stub-resolv.conf" [ "$RESOLV_TARGET" = "/run/systemd/resolve/stub-resolv.conf" ]

# r[verify image.base.console-font]
check "console-setup config exists" test -f "$MNT/etc/default/console-setup"
if [ -f "$MNT/etc/default/console-setup" ]; then
    check "console-setup FONTFACE is Fixed" grep -q 'FONTFACE="Fixed"' "$MNT/etc/default/console-setup"
    check "console-setup FONTSIZE is 8x16" grep -q 'FONTSIZE="8x16"' "$MNT/etc/default/console-setup"
fi

# r[verify image.base.login-banner]
check "/etc/issue exists" test -f "$MNT/etc/issue"
if [ -f "$MNT/etc/issue" ]; then
    check "/etc/issue includes IPv4 escape" grep -qF '\4' "$MNT/etc/issue"
    check "/etc/issue includes IPv6 escape" grep -qF '\6' "$MNT/etc/issue"
fi

# r[verify image.base.network+2]
check "netplan config exists" test -f "$MNT/etc/netplan/01-all-en-dhcp.yaml"
NETPLAN_MODE="$(stat -c%a "$MNT/etc/netplan/01-all-en-dhcp.yaml" 2>/dev/null || true)"
check "netplan config has mode 600" [ "$NETPLAN_MODE" = "600" ]
check "netplan config matches en*" grep -q 'name:.*"en\*"' "$MNT/etc/netplan/01-all-en-dhcp.yaml"
check "netplan config enables dhcp4" grep -q 'dhcp4:.*true' "$MNT/etc/netplan/01-all-en-dhcp.yaml"

# r[verify image.boot.dracut]
check "kernel exists in /boot" ls "$MNT"/boot/vmlinuz-* >/dev/null 2>&1
check "initramfs exists in /boot" ls "$MNT"/boot/initrd.img-* >/dev/null 2>&1

if [ "$VARIANT" = "pi" ]; then
    # r[verify image.boot.pi-firmware] r[verify image.boot.pi-cmdline]
    check "Pi config.txt exists" test -f "$MNT/boot/firmware/config.txt"
    check "Pi cmdline.txt exists" test -f "$MNT/boot/firmware/cmdline.txt"
    if [ -f "$MNT/boot/firmware/config.txt" ]; then
        # r[verify image.boot.pi-tryboot-rollback]
        check "config.txt sets os_prefix=current/" grep -q '^os_prefix=current/' "$MNT/boot/firmware/config.txt"
        check "config.txt sets os_prefix=new/ under [tryboot]" grep -q '^os_prefix=new/' "$MNT/boot/firmware/config.txt"
        # r[verify image.boot.pi-uart]
        check "config.txt enables UART" grep -q '^enable_uart=1' "$MNT/boot/firmware/config.txt"
        # r[verify image.boot.pi-peripherals]
        check "config.txt enables I2C" grep -q '^dtparam=i2c_arm=on' "$MNT/boot/firmware/config.txt"
        check "config.txt enables SPI" grep -q '^dtparam=spi=on' "$MNT/boot/firmware/config.txt"
        # r[verify image.boot.pi-tpm-overlay]
        check "config.txt enables tpm-slb9670 overlay" grep -q '^dtoverlay=tpm-slb9670' "$MNT/boot/firmware/config.txt"
        # r[verify image.boot.pi-pcie-gen3]
        check "config.txt sets PCIe gen 3" grep -q '^dtparam=pciex1_gen=3' "$MNT/boot/firmware/config.txt"
        check "config.txt disables splash" grep -q '^disable_splash=1' "$MNT/boot/firmware/config.txt"
    fi
    if [ -f "$MNT/boot/firmware/cmdline.txt" ]; then
        check "cmdline.txt references LUKS-mapped root" grep -q 'root=/dev/mapper/root' "$MNT/boot/firmware/cmdline.txt"
        check "cmdline.txt sets btrfs subvol" grep -q 'subvol=@' "$MNT/boot/firmware/cmdline.txt"
        # r[verify image.boot.pi-uart]
        check "cmdline.txt sets serial0 console" grep -q 'console=serial0,115200' "$MNT/boot/firmware/cmdline.txt"
    fi
    # r[verify image.boot.pi-tryboot-rollback]
    check "autoboot.txt enables tryboot_a_b" grep -q '^tryboot_a_b=1' "$MNT/boot/firmware/autoboot.txt"
    check "current/state is good" sh -c "test \"\$(cat '$MNT/boot/firmware/current/state' 2>/dev/null)\" = good"
    # r[verify image.boot.pi-firmware-update]
    check "/boot/firmware/current/ has kernel" test -f "$MNT/boot/firmware/current/vmlinuz"
    check "/boot/firmware/current/ has initramfs" test -f "$MNT/boot/firmware/current/initrd.img"
    check "/boot/firmware/current/ has Pi 5 DTB" test -f "$MNT/boot/firmware/current/bcm2712-rpi-5-b.dtb"
    check "kernel postinst hook installed (zz-flash-kernel)" test -x "$MNT/etc/kernel/postinst.d/zz-flash-kernel"
    # Legacy hand-rolled hook + helper must not be present (replaced by flash-kernel).
    check_not "no legacy bes-pi-firmware-update helper" test -e "$MNT/usr/local/sbin/bes-pi-firmware-update"
    check_not "no legacy zz-bes-pi-firmware hook" test -e "$MNT/etc/kernel/postinst.d/zz-bes-pi-firmware"
    # No GRUB on pi.
    check_not "no /boot/grub on pi" test -d "$MNT/boot/grub"
    check_not "no GRUB EFI binary on pi (BOOTAA64.EFI)" test -f "$MNT/boot/firmware/EFI/BOOT/BOOTAA64.EFI"
    # r[verify image.boot.pi-peripherals]
    check "i2c-tools installed (i2cdetect)" test -x "$MNT/usr/sbin/i2cdetect"
    # r[verify image.boot.pi-power-key]
    check "logind power-key drop-in installed" test -f "$MNT/etc/systemd/logind.conf.d/50-bes-power.conf"
    if [ -f "$MNT/etc/systemd/logind.conf.d/50-bes-power.conf" ]; then
        check "logind power-key set to poweroff" grep -q '^HandlePowerKey=poweroff' "$MNT/etc/systemd/logind.conf.d/50-bes-power.conf"
    fi
else
    # r[verify image.boot.grub-install]
    check "GRUB config exists" test -f "$MNT/boot/grub/grub.cfg"
    if [ "$ARCH" = "amd64" ]; then
        check "EFI bootloader exists (BOOTX64.EFI)" test -f "$MNT/boot/efi/EFI/BOOT/BOOTX64.EFI"
    else
        check "EFI bootloader exists (BOOTAA64.EFI)" test -f "$MNT/boot/efi/EFI/BOOT/BOOTAA64.EFI"
    fi
fi

# r[verify image.boot.grub-uuids]
if [ "$VARIANT" != "pi" ] && [ -f "$MNT/boot/grub/grub.cfg" ]; then
    GRUB_CFG="$MNT/boot/grub/grub.cfg"

    ACTUAL_ROOT_UUID="$(blkid -o value -s UUID "$BTRFS_DEV" 2>/dev/null || true)"
    if [ -n "$ACTUAL_ROOT_UUID" ]; then
        check "grub.cfg references actual root UUID ($ACTUAL_ROOT_UUID)" \
            grep -q "$ACTUAL_ROOT_UUID" "$GRUB_CFG"
    else
        fail "grub.cfg root UUID check (could not read root filesystem UUID)"
    fi

    ACTUAL_BOOT_UUID="$(blkid -o value -s UUID "$BOOT_PART" 2>/dev/null || true)"
    if [ -n "$ACTUAL_BOOT_UUID" ]; then
        check "grub.cfg references actual boot UUID ($ACTUAL_BOOT_UUID)" \
            grep -q "$ACTUAL_BOOT_UUID" "$GRUB_CFG"
    else
        fail "grub.cfg boot UUID check (could not read boot filesystem UUID)"
    fi

    # Verify every search --fs-uuid in grub.cfg points to a real filesystem
    GRUB_SEARCH_UUIDS="$(grep -oP '(?<=search\s--no-floppy\s--fs-uuid\s--set=root\s)\S+' "$GRUB_CFG" 2>/dev/null | sort -u || true)"
    if [ -n "$GRUB_SEARCH_UUIDS" ]; then
        while IFS= read -r search_uuid; do
            if [ "$search_uuid" = "$ACTUAL_ROOT_UUID" ] || [ "$search_uuid" = "$ACTUAL_BOOT_UUID" ]; then
                pass "grub.cfg search UUID $search_uuid matches a real filesystem"
            else
                fail "grub.cfg search UUID $search_uuid matches a real filesystem"
            fi
        done <<< "$GRUB_SEARCH_UUIDS"
    fi

    # Verify every root=UUID= kernel param points to the actual root
    GRUB_ROOT_PARAMS="$(grep -oP '(?<=root=UUID=)\S+' "$GRUB_CFG" 2>/dev/null | sort -u || true)"
    if [ -n "$GRUB_ROOT_PARAMS" ]; then
        while IFS= read -r root_param_uuid; do
            check "grub.cfg root=UUID=$root_param_uuid matches root filesystem" \
                [ "$root_param_uuid" = "$ACTUAL_ROOT_UUID" ]
        done <<< "$GRUB_ROOT_PARAMS"
    fi
fi

# r[verify image.credentials.no-root-ssh]
check "SSH no-root config exists" test -f "$MNT/etc/ssh/sshd_config.d/50-bes-no-root.conf"
check "SSH no-root config correct" grep -q "PermitRootLogin no" "$MNT/etc/ssh/sshd_config.d/50-bes-no-root.conf"

# r[verify image.credentials.ssh-password-auth]
check "SSH password-auth config exists" test -f "$MNT/etc/ssh/sshd_config.d/50-bes-password-auth.conf"
if [ "$VARIANT" = "cloud" ]; then
    check "SSH password auth disabled for cloud" grep -q "PasswordAuthentication no" "$MNT/etc/ssh/sshd_config.d/50-bes-password-auth.conf"
else
    check "SSH password auth enabled for $VARIANT" grep -q "PasswordAuthentication yes" "$MNT/etc/ssh/sshd_config.d/50-bes-password-auth.conf"
fi

# r[verify image.credentials.no-host-keys+2]
HOST_KEY_COUNT="$(find "$MNT/etc/ssh" -name 'ssh_host_*' 2>/dev/null | wc -l)"
check "no SSH host keys in image" test "$HOST_KEY_COUNT" -eq 0

# r[verify image.credentials.host-key-regen]
check "bes-ssh-keygen.service exists" test -f "$MNT/etc/systemd/system/bes-ssh-keygen.service"
check "bes-ssh-keygen.service is enabled" test -L "$MNT/etc/systemd/system/multi-user.target.wants/bes-ssh-keygen.service"

# r[verify image.cloud-init.no-hostname-file]
check "cloud-init BES config exists" test -f "$MNT/etc/cloud/cloud.cfg.d/99-bes.cfg"
check "cloud-init has no hostname_file setting" grep -q "create_hostname_file: false" "$MNT/etc/cloud/cloud.cfg.d/99-bes.cfg"

# r[verify image.cloud-init.no-network]
check_not "installer network config absent" test -f "$MNT/etc/cloud/cloud.cfg.d/90-installer-network.cfg"

check_not "unminimize prompt absent" test -f "$MNT/etc/update-motd.d/60-unminimize"

# ============================================================
# Snapper configuration
# ============================================================
echo ""
echo "--- Snapper ---"

# r[verify image.snapper.root]
SNAPPER_ROOT_CFG="$MNT/etc/snapper/configs/root"
check "snapper root config exists" test -f "$SNAPPER_ROOT_CFG"
if [ -f "$SNAPPER_ROOT_CFG" ]; then
    check "snapper root: TIMELINE_CREATE=yes" grep -q '^TIMELINE_CREATE="yes"' "$SNAPPER_ROOT_CFG"
    check "snapper root: TIMELINE_CLEANUP=yes" grep -q '^TIMELINE_CLEANUP="yes"' "$SNAPPER_ROOT_CFG"
    check "snapper root: NUMBER_CLEANUP=yes" grep -q '^NUMBER_CLEANUP="yes"' "$SNAPPER_ROOT_CFG"
    check "snapper root: NUMBER_LIMIT=10" grep -q '^NUMBER_LIMIT="10"' "$SNAPPER_ROOT_CFG"
    check "snapper root: TIMELINE_LIMIT_HOURLY=6" grep -q '^TIMELINE_LIMIT_HOURLY="6"' "$SNAPPER_ROOT_CFG"
    check "snapper root: TIMELINE_LIMIT_DAILY=0" grep -q '^TIMELINE_LIMIT_DAILY="0"' "$SNAPPER_ROOT_CFG"
    check "snapper root: TIMELINE_LIMIT_WEEKLY=0" grep -q '^TIMELINE_LIMIT_WEEKLY="0"' "$SNAPPER_ROOT_CFG"
    check "snapper root: TIMELINE_LIMIT_MONTHLY=0" grep -q '^TIMELINE_LIMIT_MONTHLY="0"' "$SNAPPER_ROOT_CFG"
    check "snapper root: TIMELINE_LIMIT_YEARLY=0" grep -q '^TIMELINE_LIMIT_YEARLY="0"' "$SNAPPER_ROOT_CFG"
fi

# r[verify image.snapper.postgres]
SNAPPER_PG_CFG="$MNT/etc/snapper/configs/postgres"
check "snapper postgres config exists" test -f "$SNAPPER_PG_CFG"
if [ -f "$SNAPPER_PG_CFG" ]; then
    check "snapper postgres: TIMELINE_CREATE=yes" grep -q '^TIMELINE_CREATE="yes"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_CLEANUP=yes" grep -q '^TIMELINE_CLEANUP="yes"' "$SNAPPER_PG_CFG"
    check "snapper postgres: NUMBER_CLEANUP=yes" grep -q '^NUMBER_CLEANUP="yes"' "$SNAPPER_PG_CFG"
    check "snapper postgres: NUMBER_LIMIT=10" grep -q '^NUMBER_LIMIT="10"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_LIMIT_HOURLY=6" grep -q '^TIMELINE_LIMIT_HOURLY="6"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_LIMIT_DAILY=0" grep -q '^TIMELINE_LIMIT_DAILY="0"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_LIMIT_WEEKLY=0" grep -q '^TIMELINE_LIMIT_WEEKLY="0"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_LIMIT_MONTHLY=0" grep -q '^TIMELINE_LIMIT_MONTHLY="0"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_LIMIT_YEARLY=0" grep -q '^TIMELINE_LIMIT_YEARLY="0"' "$SNAPPER_PG_CFG"
fi

# ============================================================
# UFW firewall rules
# ============================================================
echo ""
echo "--- Firewall Rules ---"

UFW_RULES_DIR="$MNT/etc/ufw"
UFW_USER_RULES="$UFW_RULES_DIR/user.rules"
UFW_USER6_RULES="$UFW_RULES_DIR/user6.rules"

# r[verify image.firewall.policy]
UFW_DEFAULT="$UFW_RULES_DIR/ufw.conf"
if [ -f "$UFW_DEFAULT" ]; then
    check "ufw is enabled in config" grep -q '^ENABLED=yes' "$UFW_DEFAULT"
fi
UFW_DEFAULTS="$UFW_RULES_DIR/default"
if [ -f "$UFW_DEFAULTS" ] || [ -f "$MNT/etc/default/ufw" ]; then
    DEFAULTS_FILE="$UFW_DEFAULTS"
    [ -f "$DEFAULTS_FILE" ] || DEFAULTS_FILE="$MNT/etc/default/ufw"
    check "ufw default incoming=deny" grep -q 'DEFAULT_INPUT_POLICY="DROP"' "$DEFAULTS_FILE"
    check "ufw default outgoing=allow" grep -q 'DEFAULT_OUTPUT_POLICY="ACCEPT"' "$DEFAULTS_FILE"
    check "ufw default forward=allow" grep -q 'DEFAULT_FORWARD_POLICY="ACCEPT"' "$DEFAULTS_FILE"
fi

# r[verify image.firewall.ssh]
if [ -f "$UFW_USER_RULES" ]; then
    check "ufw allows 22/tcp" grep -q '\-p tcp --dport 22' "$UFW_USER_RULES"
fi

# r[verify image.firewall.http]
if [ -f "$UFW_USER_RULES" ]; then
    check "ufw allows 80/tcp" grep -q '\-p tcp --dport 80' "$UFW_USER_RULES"
    check "ufw allows 443/tcp" grep -q '\-p tcp --dport 443' "$UFW_USER_RULES"
    check "ufw allows 443/udp" grep -q '\-p udp --dport 443' "$UFW_USER_RULES"
fi

# r[verify image.packages.bes-tools]
check "bes-tools signing key installed" test -f "$MNT/etc/apt/keyrings/bes-tools.gpg"
check "bes-tools apt repo configured" test -f "$MNT/etc/apt/sources.list.d/bes-tools.list"
check "bes-tools apt pin configured" test -f "$MNT/etc/apt/preferences.d/99-bes-tools"

# r[verify image.packages.kopia]
check "Kopia signing key installed" test -f "$MNT/etc/apt/keyrings/kopia-keyring.gpg"
check "Kopia apt repo configured" test -f "$MNT/etc/apt/sources.list.d/kopia.list"
check "Kopia apt pin configured" test -f "$MNT/etc/apt/preferences.d/99-kopia"

# r[verify image.packages.tailscale]
check "Tailscale signing key installed" test -f "$MNT/usr/share/keyrings/tailscale-archive-keyring.gpg"
check "Tailscale apt repo configured" test -f "$MNT/etc/apt/sources.list.d/tailscale.list"

# r[verify image.packages.tailscale]
check "Tailscale apt prefer configured" test -f "$MNT/etc/apt/preferences.d/99-tailscale"

# r[verify image.tailscale.auto-update]
check "Tailscale weekly cron exists" test -x "$MNT/etc/cron.weekly/apt-upgrade-tailscale"

# The hostonly workaround and its force-include driver lists only apply on
# noble. On 26.04+, dracut's default mode includes hardware modules without
# explicit dracut.conf.d overrides.
if [ "$VARIANT" = "pi" ]; then
    # Pi always uses the portable-image config (hostonly=no), independent of
    # suite — the hardware-drivers list is x86-server-leaning and many of
    # those modules don't exist in linux-raspi.
    check "dracut portable-image config exists on pi" test -f "$MNT/etc/dracut.conf.d/01-portable-image.conf"
    check "dracut portable-image config sets hostonly=no on pi" grep -q 'hostonly="no"' "$MNT/etc/dracut.conf.d/01-portable-image.conf"
    check_not "no dracut hostonly fix config on pi" test -f "$MNT/etc/dracut.conf.d/01-fix-hostonly.conf"
    check_not "no dracut hardware-drivers config on pi" test -f "$MNT/etc/dracut.conf.d/03-hardware-drivers.conf"
    check_not "no dracut cloud-drivers config on pi" test -f "$MNT/etc/dracut.conf.d/04-cloud-drivers.conf"
elif [ "$SUITE" = "noble" ]; then
    # r[verify image.boot.dracut]
    check "dracut hostonly config exists" test -f "$MNT/etc/dracut.conf.d/01-fix-hostonly.conf"
    check "dracut hostonly=yes" grep -q 'hostonly="yes"' "$MNT/etc/dracut.conf.d/01-fix-hostonly.conf"

    # r[verify image.boot.hardware-drivers+3]
    check "dracut hardware-drivers config exists" test -f "$MNT/etc/dracut.conf.d/03-hardware-drivers.conf"
    HWDRV="$MNT/etc/dracut.conf.d/03-hardware-drivers.conf"
    check "dracut hardware-drivers has nvme" grep -wq 'nvme' "$HWDRV"
    check "dracut hardware-drivers has nvme_core" grep -wq 'nvme_core' "$HWDRV"
    check "dracut hardware-drivers has ahci" grep -wq 'ahci' "$HWDRV"
    check "dracut hardware-drivers has megaraid_sas" grep -wq 'megaraid_sas' "$HWDRV"
    check "dracut hardware-drivers has mpt3sas" grep -wq 'mpt3sas' "$HWDRV"
    check "dracut hardware-drivers has virtio_blk" grep -wq 'virtio_blk' "$HWDRV"
    check "dracut hardware-drivers has virtio_scsi" grep -wq 'virtio_scsi' "$HWDRV"
    check "dracut hardware-drivers has virtio_net" grep -wq 'virtio_net' "$HWDRV"
    check "dracut hardware-drivers has virtio_pci" grep -wq 'virtio_pci' "$HWDRV"
    check "dracut hardware-drivers has e1000e" grep -wq 'e1000e' "$HWDRV"
    check "dracut hardware-drivers has igb" grep -wq 'igb' "$HWDRV"
    check "dracut hardware-drivers has ixgbe" grep -wq 'ixgbe' "$HWDRV"
    check "dracut hardware-drivers has i40e" grep -wq 'i40e' "$HWDRV"
    check "dracut hardware-drivers has ice" grep -wq 'ice' "$HWDRV"
    check "dracut hardware-drivers has bnxt_en" grep -wq 'bnxt_en' "$HWDRV"
    check "dracut hardware-drivers has tg3" grep -wq 'tg3' "$HWDRV"
    check "dracut hardware-drivers has mlx5_core" grep -wq 'mlx5_core' "$HWDRV"
    check "dracut hardware-drivers has usb_storage" grep -wq 'usb_storage' "$HWDRV"
    check "dracut hardware-drivers has uas" grep -wq 'uas' "$HWDRV"
    check "dracut hardware-drivers has hv_storvsc" grep -wq 'hv_storvsc' "$HWDRV"
    check "dracut hardware-drivers has hv_netvsc" grep -wq 'hv_netvsc' "$HWDRV"
    check "dracut hardware-drivers has hv_vmbus" grep -wq 'hv_vmbus' "$HWDRV"

    # r[verify image.boot.cloud-drivers+5]
    if [ "$VARIANT" = "cloud" ]; then
        check "dracut cloud-drivers config exists" test -f "$MNT/etc/dracut.conf.d/04-cloud-drivers.conf"
        CLOUDDRV="$MNT/etc/dracut.conf.d/04-cloud-drivers.conf"
        check "dracut cloud-drivers has ena" grep -wq 'ena' "$CLOUDDRV"
        check "dracut cloud-drivers has xen_blkfront" grep -wq 'xen_blkfront' "$CLOUDDRV"
        check "dracut cloud-drivers has gve" grep -wq 'gve' "$CLOUDDRV"
    else
        check_not "no cloud-drivers config for $VARIANT variant" test -f "$MNT/etc/dracut.conf.d/04-cloud-drivers.conf"
    fi
else
    # r[verify image.boot.dracut]
    check_not "no dracut hostonly fix config on non-noble" test -f "$MNT/etc/dracut.conf.d/01-fix-hostonly.conf"
    # r[verify image.boot.hardware-drivers+3]
    check_not "no dracut hardware-drivers config on non-noble" test -f "$MNT/etc/dracut.conf.d/03-hardware-drivers.conf"
    # r[verify image.boot.cloud-drivers+5]
    check_not "no dracut cloud-drivers config on non-noble" test -f "$MNT/etc/dracut.conf.d/04-cloud-drivers.conf"
    # r[verify image.boot.dracut]: portable image config supplies hostonly=no
    check "dracut portable-image config exists on non-noble" test -f "$MNT/etc/dracut.conf.d/01-portable-image.conf"
    check "dracut portable-image config sets hostonly=no" grep -q 'hostonly="no"' "$MNT/etc/dracut.conf.d/01-portable-image.conf"
fi

# GRUB doesn't apply to the pi variant (it boots via the Pi 5 firmware).
if [ "$VARIANT" != "pi" ]; then
    # r[verify image.boot.grub-timeout]
    check "GRUB timeout is 5" grep -q '^GRUB_TIMEOUT=5' "$MNT/etc/default/grub"
    check "GRUB timeout style is menu" grep -q '^GRUB_TIMEOUT_STYLE=menu' "$MNT/etc/default/grub"
    check "GRUB recordfail timeout is 5" grep -q '^GRUB_RECORDFAIL_TIMEOUT=5' "$MNT/etc/default/grub"

    # r[verify image.boot.grub-cmdline]
    check "GRUB cmdline has noresume" grep -q 'noresume' "$MNT/etc/default/grub"

    # r[verify image.boot.cloud-console]
    if [ "$VARIANT" = "cloud" ]; then
        check "GRUB cmdline has serial console for cloud" grep -q 'console=ttyS0,115200n8' "$MNT/etc/default/grub"
    else
        check_not "GRUB cmdline has no serial console for metal" grep -q 'console=ttyS0' "$MNT/etc/default/grub"
    fi
fi

# r[verify image.credentials.ubuntu-user]
check "ubuntu user exists in passwd" grep -q '^ubuntu:' "$MNT/etc/passwd"
if grep '^ubuntu:' "$MNT/etc/passwd" | grep -q '/bin/bash$'; then
    pass "ubuntu user has /bin/bash shell"
else
    fail "ubuntu user has /bin/bash shell"
fi

# r[verify image.credentials.root-disabled]
if grep '^root:' "$MNT/etc/passwd" | grep -q '/sbin/nologin$'; then
    pass "root user has /sbin/nologin shell"
else
    fail "root user has /sbin/nologin shell"
fi

# ============================================================
# 5. systemd service checks
# ============================================================
echo ""
echo "--- Services ---"

# Check enabled services by looking for symlinks in .wants directories
check_service_enabled() {
    local svc="$1"
    local desc="$2"
    if find "$MNT/etc/systemd/system" -name "$svc" -type l 2>/dev/null | grep -q .; then
        pass "$desc"
    else
        fail "$desc"
    fi
}

check_service_enabled "ssh.service"                   "ssh is enabled"

# r[verify image.firewall.enabled]
check_service_enabled "ufw.service"                   "ufw is enabled"

# r[verify image.tailscale.service-enabled]
check_service_enabled "tailscaled.service"            "tailscaled is enabled"
# r[verify image.tailscale.firstboot-auth]
check_service_enabled "bes-tailscale-firstboot-auth.service" "bes-tailscale-firstboot-auth is enabled"

# r[verify image.firstboot.script]
check_service_enabled "bes-firstboot-script.service"  "bes-firstboot-script is enabled"

# r[verify image.snapper.timers]
check_service_enabled "snapper-timeline.timer"        "snapper-timeline.timer is enabled"
check_service_enabled "snapper-cleanup.timer"         "snapper-cleanup.timer is enabled"

# r[verify image.growth.service+3]
check_service_enabled "grow-root-filesystem.service"  "grow-root-filesystem is enabled"

# r[verify image.cloud-init.enabled]
# On noble, cloud-init.service is the static entry point that gets enabled into
# multi-user.target.wants. On 26.04+, the unified service was removed and
# cloud-init.target is wired in dynamically by cloud-init-generator at boot.
if [ "$SUITE" = "noble" ]; then
    check_service_enabled "cloud-init.service"            "cloud-init is enabled"
else
    check "cloud-init-generator exists" test -x "$MNT/usr/lib/systemd/system-generators/cloud-init-generator"
    check "cloud-init.target.wants populated" test -d "$MNT/etc/systemd/system/cloud-init.target.wants"
fi

# r[verify image.packages.chrony]
check_service_enabled "chrony.service"                "chrony is enabled"
check "chrony binary exists" test -x "$MNT/usr/sbin/chronyd"
# systemd-timesyncd must not be enabled — chrony is the time daemon.
if [ -L "$MNT/etc/systemd/system/multi-user.target.wants/systemd-timesyncd.service" ] \
        || [ -L "$MNT/etc/systemd/system/sysinit.target.wants/systemd-timesyncd.service" ] \
        || [ -L "$MNT/etc/systemd/system/dbus-org.freedesktop.timesync1.service" ]; then
    fail "systemd-timesyncd is not enabled"
else
    pass "systemd-timesyncd is not enabled"
fi

case "$VARIANT" in
    metal|pi)
        # r[verify image.luks.keyfile]
        check "LUKS empty keyfile exists" test -f "$MNT/etc/luks/empty-keyfile"
        KEYFILE_MODE="$(stat -c%a "$MNT/etc/luks/empty-keyfile" 2>/dev/null || true)"
        check "LUKS empty keyfile has mode 000" [ "$KEYFILE_MODE" = "0" ]

        # r[verify image.luks.crypttab]
        check "crypttab exists" test -f "$MNT/etc/crypttab"
        check "crypttab references by-partlabel/root" grep -q "by-partlabel/root" "$MNT/etc/crypttab"
        check "crypttab has force option" grep -q "force" "$MNT/etc/crypttab"

        # r[verify image.luks.keyfile]
        check "dracut LUKS keyfile config exists" test -f "$MNT/etc/dracut.conf.d/02-luks-keyfile.conf"
        ;;
    cloud)
        check_not "no crypttab for cloud variant" test -f "$MNT/etc/crypttab"
        ;;
esac

# ============================================================
# 6. fstab validation
# ============================================================
echo ""
echo "--- fstab ---"

FSTAB="$MNT/etc/fstab"
if [ -f "$FSTAB" ]; then
    check "fstab has / mount" grep -qE '^\S+\s+/\s+btrfs\s+.*subvol=@' "$FSTAB"
    check "fstab has /var/lib/postgresql mount" grep -qE '^\S+\s+/var/lib/postgresql\s+btrfs\s+.*subvol=@postgres' "$FSTAB"
    check "fstab has /boot mount" grep -qE '^\S+\s+/boot\s+ext4' "$FSTAB"
    if [ "$VARIANT" = "pi" ]; then
        check "fstab has /boot/firmware mount" grep -qE '^\S+\s+/boot/firmware\s+vfat' "$FSTAB"
    else
        check "fstab has /boot/efi mount" grep -qE '^\S+\s+/boot/efi\s+vfat' "$FSTAB"
    fi

    # r[verify image.btrfs.compression]
    if grep -E '^\S+\s+/\s' "$FSTAB" | grep -q 'compress=zstd:6'; then
        pass "fstab has compress=zstd:6 on root"
    else
        fail "fstab has compress=zstd:6 on root"
    fi

    # r[verify image.partition.count]
    if grep -qE '^\S+\s+\S+\s+swap\s' "$FSTAB"; then
        fail "fstab has no swap entries"
    else
        pass "fstab has no swap entries"
    fi

    case "$VARIANT" in
        metal|pi)
            if grep -E '^\S+\s+/\s' "$FSTAB" | grep -q '/dev/mapper/root'; then
                pass "fstab uses /dev/mapper/root for / ($VARIANT)"
            else
                fail "fstab uses /dev/mapper/root for / ($VARIANT)"
            fi
            if grep -E '^\S+\s+/var/lib/postgresql\s' "$FSTAB" | grep -q '/dev/mapper/root'; then
                pass "fstab uses /dev/mapper/root for pg ($VARIANT)"
            else
                fail "fstab uses /dev/mapper/root for pg ($VARIANT)"
            fi
            ;;
        cloud)
            if grep -E '^\S+\s+/\s' "$FSTAB" | grep -q 'by-partlabel/root'; then
                pass "fstab uses by-partlabel/root for / (cloud)"
            else
                fail "fstab uses by-partlabel/root for / (cloud)"
            fi
            if grep -E '^\S+\s+/var/lib/postgresql\s' "$FSTAB" | grep -q 'by-partlabel/root'; then
                pass "fstab uses by-partlabel/root for pg (cloud)"
            else
                fail "fstab uses by-partlabel/root for pg (cloud)"
            fi
            ;;
    esac
    if grep -E '^\S+\s+/boot\s' "$FSTAB" | grep -q 'by-partlabel/xboot'; then
        pass "fstab uses by-partlabel/xboot for /boot"
    else
        fail "fstab uses by-partlabel/xboot for /boot"
    fi
    if [ "$VARIANT" = "pi" ]; then
        if grep -E '^\S+\s+/boot/firmware\s' "$FSTAB" | grep -q 'by-partlabel/firmware'; then
            pass "fstab uses by-partlabel/firmware for /boot/firmware"
        else
            fail "fstab uses by-partlabel/firmware for /boot/firmware"
        fi
    else
        if grep -E '^\S+\s+/boot/efi\s' "$FSTAB" | grep -q 'by-partlabel/efi'; then
            pass "fstab uses by-partlabel/efi for /boot/efi"
        else
            fail "fstab uses by-partlabel/efi for /boot/efi"
        fi
    fi
else
    fail "fstab exists"
fi

# ============================================================
# 7. Package checks
# ============================================================
echo ""
echo "--- Packages ---"

# Use the image's own dpkg-query via chroot so we don't depend on host tools
# or regex parsing of the status file.
if [ -x "$MNT/usr/bin/dpkg-query" ]; then
    # Bind-mount /proc so dpkg-query doesn't complain
    mount -t proc proc "$MNT/proc" 2>/dev/null || true

    source "$PACKAGES_FILE"
    for pkg in "${PACKAGES[@]}"; do
        # shellcheck disable=SC2016 # ${Status} is a dpkg format string, not a bash variable
        if chroot "$MNT" dpkg-query -W -f='${Status}\n' "$pkg" 2>/dev/null | grep -q "install ok installed"; then
            pass "package '$pkg' is installed"
        else
            fail "package '$pkg' is installed"
        fi
    done

    # netavark and aardvark-dns are expected as dependencies of podman from the bes-tools repo
    for dep_pkg in netavark aardvark-dns; do
        # shellcheck disable=SC2016 # ${Status} is a dpkg format string, not a bash variable
        if chroot "$MNT" dpkg-query -W -f='${Status}\n' "$dep_pkg" 2>/dev/null | grep -q "install ok installed"; then
            pass "podman dependency '$dep_pkg' is installed"
        else
            fail "podman dependency '$dep_pkg' is installed"
        fi
    done

    # caddy is never pre-installed: consumers install it themselves from the
    # OS archive when they need it.
    # shellcheck disable=SC2016 # ${Status} is a dpkg format string, not a bash variable
    if chroot "$MNT" dpkg-query -W -f='${Status}\n' caddy 2>/dev/null | grep -q "install ok installed"; then
        fail "caddy must not be pre-installed"
    else
        pass "caddy is not pre-installed"
    fi
    # r[verify image.packages.podman]
    check_pkg_version podman   5.0.0  "image.packages.podman"
    # r[verify image.packages.kopia]
    check_pkg_version kopia    0.22.0 "image.packages.kopia"
    # r[verify image.packages.tailscale]
    check_pkg_version tailscale 1.92.0 "image.packages.tailscale"
    # r[verify image.packages.bestool+2]
    check_pkg_version bestool  1.4.0  "image.packages.bestool"

    # r[verify image.boot.dracut]
    # shellcheck disable=SC2016
    if chroot "$MNT" dpkg-query -W -f='${Status}\n' initramfs-tools 2>/dev/null | grep -q "install ok installed"; then
        fail "initramfs-tools is NOT installed (dracut should replace it)"
    else
        pass "initramfs-tools is NOT installed (dracut should replace it)"
    fi
    # shellcheck disable=SC2016
    if chroot "$MNT" dpkg-query -W -f='${Status}\n' dracut 2>/dev/null | grep -q "install ok installed"; then
        pass "dracut is installed"
    else
        fail "dracut is installed"
    fi

    umount "$MNT/proc" 2>/dev/null || true
else
    fail "dpkg-query not found in image — cannot verify packages"
fi

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

exit 0
