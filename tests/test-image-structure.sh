#!/bin/bash
# Loopback-mount a built image and verify its structure without booting.
# This runs in CI without KVM.
#
# Usage: test-image-structure.sh <image.raw> <variant> <arch>
#   variant: metal | cloud
#   arch:    amd64 | arm64
set -euo pipefail

IMAGE="${1:?Usage: $0 <image.raw> <variant> <arch>}"
VARIANT="${2:?Usage: $0 <image.raw> <variant> <arch>}"
ARCH="${3:?Usage: $0 <image.raw> <variant> <arch>}"

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
    [ "$BOOT_MOUNTED" -eq 1 ] && umount "$MNT/boot" 2>/dev/null
    [ "$PG_MOUNTED" -eq 1 ]   && umount "$MNT/var/lib/postgresql" 2>/dev/null
    [ "$ROOT_MOUNTED" -eq 1 ] && umount "$MNT" 2>/dev/null
    if [ "$VARIANT" = "metal" ]; then
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

# r[verify image.partition.efi]
EFI_LABEL="$(get_part_label 1)"
EFI_TYPE="$(get_part_type 1)"
check "partition 1 label is 'efi'" [ "$EFI_LABEL" = "efi" ]
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
if [ "$VARIANT" = "metal" ]; then
    ROOT_FSTYPE="$(blkid -o value -s TYPE "$ROOT_PART" 2>/dev/null || true)"
    check "root partition is crypto_LUKS (metal)" [ "$ROOT_FSTYPE" = "crypto_LUKS" ]

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

# r[verify image.btrfs.format]
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

mkdir -p "$MNT/boot/efi"
mount "$EFI_PART" "$MNT/boot/efi"
EFI_MOUNTED=1

# r[verify image.base.debootstrap]
check "/etc/fstab exists" test -f "$MNT/etc/fstab"

# r[verify image.variant.types]
check "/etc/bes/image-variant exists" test -f "$MNT/etc/bes/image-variant"

# r[verify image.tailscale.ts-up]
check "/usr/local/bin/ts-up exists" test -x "$MNT/usr/local/bin/ts-up"

# r[verify image.growth.script]
check "/usr/local/bin/grow-root-filesystem exists" test -x "$MNT/usr/local/bin/grow-root-filesystem"

# r[verify image.growth.service]
check "/etc/systemd/system/grow-root-filesystem.service exists" test -f "$MNT/etc/systemd/system/grow-root-filesystem.service"

# r[verify image.variant.types]
ACTUAL_VARIANT="$(cat "$MNT/etc/bes/image-variant" 2>/dev/null || true)"
check "image-variant contains '$VARIANT'" [ "$ACTUAL_VARIANT" = "$VARIANT" ]

# r[verify image.base.machine-id] r[verify image.cloud-init.no-machineid]
MACHINE_ID_SIZE="$(stat -c%s "$MNT/etc/machine-id" 2>/dev/null || echo "missing")"
check "/etc/machine-id is empty (size=0)" [ "$MACHINE_ID_SIZE" = "0" ]

# r[verify image.base.resolver]
check "/etc/resolv.conf is a symlink" test -L "$MNT/etc/resolv.conf"
RESOLV_TARGET="$(readlink "$MNT/etc/resolv.conf" 2>/dev/null || true)"
check "/etc/resolv.conf points to stub-resolv.conf" [ "$RESOLV_TARGET" = "/run/systemd/resolve/stub-resolv.conf" ]

# r[verify image.boot.dracut]
check "kernel exists in /boot" ls "$MNT"/boot/vmlinuz-* >/dev/null 2>&1
check "initramfs exists in /boot" ls "$MNT"/boot/initrd.img-* >/dev/null 2>&1

# r[verify image.boot.grub-install]
check "GRUB config exists" test -f "$MNT/boot/grub/grub.cfg"
if [ "$ARCH" = "amd64" ]; then
    check "EFI bootloader exists (BOOTX64.EFI)" test -f "$MNT/boot/efi/EFI/BOOT/BOOTX64.EFI"
else
    check "EFI bootloader exists (BOOTAA64.EFI)" test -f "$MNT/boot/efi/EFI/BOOT/BOOTAA64.EFI"
fi

# r[verify image.credentials.ssh-keys-only]
check "SSH no-password config exists" test -f "$MNT/etc/ssh/sshd_config.d/50-bes-no-password.conf"
check "SSH no-password config correct" grep -q "PasswordAuthentication no" "$MNT/etc/ssh/sshd_config.d/50-bes-no-password.conf"

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
    check "snapper root: TIMELINE_LIMIT_HOURLY=10" grep -q '^TIMELINE_LIMIT_HOURLY="10"' "$SNAPPER_ROOT_CFG"
    check "snapper root: TIMELINE_LIMIT_DAILY=7" grep -q '^TIMELINE_LIMIT_DAILY="7"' "$SNAPPER_ROOT_CFG"
    check "snapper root: TIMELINE_LIMIT_WEEKLY=4" grep -q '^TIMELINE_LIMIT_WEEKLY="4"' "$SNAPPER_ROOT_CFG"
    check "snapper root: TIMELINE_LIMIT_MONTHLY=12" grep -q '^TIMELINE_LIMIT_MONTHLY="12"' "$SNAPPER_ROOT_CFG"
fi

# r[verify image.snapper.postgres]
SNAPPER_PG_CFG="$MNT/etc/snapper/configs/postgres"
check "snapper postgres config exists" test -f "$SNAPPER_PG_CFG"
if [ -f "$SNAPPER_PG_CFG" ]; then
    check "snapper postgres: TIMELINE_CREATE=yes" grep -q '^TIMELINE_CREATE="yes"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_CLEANUP=yes" grep -q '^TIMELINE_CLEANUP="yes"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_LIMIT_HOURLY=10" grep -q '^TIMELINE_LIMIT_HOURLY="10"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_LIMIT_DAILY=7" grep -q '^TIMELINE_LIMIT_DAILY="7"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_LIMIT_WEEKLY=4" grep -q '^TIMELINE_LIMIT_WEEKLY="4"' "$SNAPPER_PG_CFG"
    check "snapper postgres: TIMELINE_LIMIT_MONTHLY=12" grep -q '^TIMELINE_LIMIT_MONTHLY="12"' "$SNAPPER_PG_CFG"
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
    check "ufw allows 22/tcp" grep -q '22/tcp' "$UFW_USER_RULES"
fi

# r[verify image.firewall.http]
if [ -f "$UFW_USER_RULES" ]; then
    check "ufw allows 80/tcp" grep -q '80/tcp' "$UFW_USER_RULES"
    check "ufw allows 443/tcp" grep -q '443/tcp' "$UFW_USER_RULES"
fi
if [ -f "$UFW_USER6_RULES" ] || [ -f "$UFW_USER_RULES" ]; then
    RULES_FOR_UDP="${UFW_USER_RULES}"
    check "ufw allows 443/udp" grep -q '443/udp' "$RULES_FOR_UDP"
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

# r[verify image.boot.dracut]
check "dracut hostonly config exists" test -f "$MNT/etc/dracut.conf.d/01-fix-hostonly-noble.conf"
check "dracut hostonly=yes" grep -q 'hostonly="yes"' "$MNT/etc/dracut.conf.d/01-fix-hostonly-noble.conf"

# r[verify image.boot.grub-timeout]
check "GRUB timeout is 5" grep -q '^GRUB_TIMEOUT=5' "$MNT/etc/default/grub"
check "GRUB timeout style is menu" grep -q '^GRUB_TIMEOUT_STYLE=menu' "$MNT/etc/default/grub"
check "GRUB recordfail timeout is 5" grep -q '^GRUB_RECORDFAIL_TIMEOUT=5' "$MNT/etc/default/grub"

# r[verify image.boot.grub-cmdline]
check "GRUB cmdline has noresume" grep -q 'noresume' "$MNT/etc/default/grub"

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

# r[verify image.snapper.timers]
check_service_enabled "snapper-timeline.timer"        "snapper-timeline.timer is enabled"
check_service_enabled "snapper-cleanup.timer"         "snapper-cleanup.timer is enabled"

# r[verify image.growth.service]
check_service_enabled "grow-root-filesystem.service"  "grow-root-filesystem is enabled"

# r[verify image.cloud-init.enabled]
check_service_enabled "cloud-init.service"            "cloud-init is enabled"

if [ "$VARIANT" = "metal" ]; then
    # r[verify image.luks.reencrypt]
    check_service_enabled "luks-reencrypt.service"    "luks-reencrypt is enabled (metal)"

    # r[verify image.tpm.service]
    check_service_enabled "setup-tpm-unlock.service"  "setup-tpm-unlock is enabled (metal)"

    # r[verify image.tpm.enrollment]
    TPM_SCRIPT="$MNT/usr/local/bin/setup-tpm-unlock"
    check "setup-tpm-unlock script exists" test -x "$TPM_SCRIPT"
    if [ -f "$TPM_SCRIPT" ]; then
        check "tpm script uses systemd-cryptenroll" grep -q 'systemd-cryptenroll' "$TPM_SCRIPT"
        check "tpm script binds to PCR 7" grep -q 'tpm2-pcrs=7' "$TPM_SCRIPT"
        check "tpm script wipes password slot" grep -q 'wipe-slot' "$TPM_SCRIPT"
        check "tpm script updates crypttab for tpm2" grep -q 'tpm2-device=auto' "$TPM_SCRIPT"
        check "tpm script regenerates initramfs" grep -q 'dracut -f' "$TPM_SCRIPT"
    fi

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
else
    check_not "no crypttab for cloud variant" test -f "$MNT/etc/crypttab"
    if find "$MNT/etc/systemd/system" -name "luks-reencrypt.service" -type l 2>/dev/null | grep -q .; then
        fail "no luks-reencrypt for cloud"
    else
        pass "no luks-reencrypt for cloud"
    fi
    if find "$MNT/etc/systemd/system" -name "setup-tpm-unlock.service" -type l 2>/dev/null | grep -q .; then
        fail "no setup-tpm-unlock for cloud"
    else
        pass "no setup-tpm-unlock for cloud"
    fi
fi

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
        check "fstab has /boot/efi mount" grep -qE '^\S+\s+/boot/efi\s+vfat' "$FSTAB"

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

        if [ "$VARIANT" = "metal" ]; then
            if grep -E '^\S+\s+/\s' "$FSTAB" | grep -q '/dev/mapper/root'; then
                pass "fstab uses /dev/mapper/root for / (metal)"
            else
                fail "fstab uses /dev/mapper/root for / (metal)"
            fi
            if grep -E '^\S+\s+/var/lib/postgresql\s' "$FSTAB" | grep -q '/dev/mapper/root'; then
                pass "fstab uses /dev/mapper/root for pg (metal)"
            else
                fail "fstab uses /dev/mapper/root for pg (metal)"
            fi
        else
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
        fi
        if grep -E '^\S+\s+/boot\s' "$FSTAB" | grep -q 'by-partlabel/xboot'; then
            pass "fstab uses by-partlabel/xboot for /boot"
        else
            fail "fstab uses by-partlabel/xboot for /boot"
        fi
        if grep -E '^\S+\s+/boot/efi\s' "$FSTAB" | grep -q 'by-partlabel/efi'; then
            pass "fstab uses by-partlabel/efi for /boot/efi"
        else
            fail "fstab uses by-partlabel/efi for /boot/efi"
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

    # r[verify image.packages.caddy]
    check_pkg_version caddy    2.10.0 "image.packages.caddy"
    # r[verify image.packages.podman]
    check_pkg_version podman   5.0.0  "image.packages.podman"
    # r[verify image.packages.kopia]
    check_pkg_version kopia    0.22.0 "image.packages.kopia"
    # r[verify image.packages.tailscale]
    check_pkg_version tailscale 1.92.0 "image.packages.tailscale"
    # r[verify image.packages.bestool]
    check_pkg_version bestool  2.0.0  "image.packages.bestool"

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
