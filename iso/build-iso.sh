#!/bin/bash
#
# Build a hybrid live installer ISO with:
#   - ISO9660 filesystem (bootable in VMs as optical media)
#   - El Torito EFI boot catalog with embedded FAT32 ESP image
#   - GPT for USB boot after dd
#   - Appended FAT32 BESCONF partition (writable on USB for bes-install.toml)
#   - Squashfs live rootfs with live-boot support
#
# The resulting .iso works in VirtualBox/QEMU as a CD, and after dd to USB
# the BESCONF partition is writable for configuration injection.
#
# Usage: build-iso.sh
#   Environment variables:
#     ARCH            - amd64 or arm64 (default: amd64)
#     OUTPUT          - output file path (default: output/<arch>/bes-installer-<arch>.iso)
#     INSTALLER_BIN   - path to the bes-installer binary
#     CLOUD_IMAGE     - path to the cloud image (.raw or .raw.zst) to extract partitions from
#     UBUNTU_SUITE    - Ubuntu suite name (default: noble)
#     UBUNTU_MIRROR   - mirror URL (auto-selected per arch if unset)
#     BESCONF_SIZE_MB - BESCONF partition size in MiB (default: 4)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

ARCH="${ARCH:-amd64}"
UBUNTU_SUITE="${UBUNTU_SUITE:-noble}"
BESCONF_SIZE_MB="${BESCONF_SIZE_MB:-4}"
BUILD_DATE="$(date -u +%Y-%m-%d)"
INSTALLER_BIN="${INSTALLER_BIN:?INSTALLER_BIN must point to the bes-installer binary}"
CLOUD_IMAGE="${CLOUD_IMAGE:?CLOUD_IMAGE must point to the cloud image (.raw or .raw.zst)}"
OUTPUT="${OUTPUT:-output/${ARCH}/bes-installer-${ARCH}.iso}"

# r[impl iso.per-arch]
case "$ARCH" in
    amd64)
        UBUNTU_MIRROR="${UBUNTU_MIRROR:-http://archive.ubuntu.com/ubuntu}"
        GRUB_TARGET="x86_64-efi"
        GRUB_EFI_NAME="BOOTX64.EFI"
        ;;
    arm64)
        UBUNTU_MIRROR="${UBUNTU_MIRROR:-http://ports.ubuntu.com/ubuntu-ports}"
        GRUB_TARGET="arm64-efi"
        GRUB_EFI_NAME="BOOTAA64.EFI"
        ;;
    *)
        echo "ERROR: ARCH must be amd64 or arm64 (got: $ARCH)"
        exit 1
        ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root"
    exit 1
fi

if [ ! -f "$INSTALLER_BIN" ]; then
    echo "ERROR: installer binary not found: $INSTALLER_BIN"
    exit 1
fi

if [ ! -f "$CLOUD_IMAGE" ]; then
    echo "ERROR: cloud image not found: $CLOUD_IMAGE"
    exit 1
fi

MISSING=()
for cmd in debootstrap mksquashfs sfdisk mkfs.vfat losetup grub-mkimage xorriso zstd jq veritysetup; do
    command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
done
if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "ERROR: missing required commands: ${MISSING[*]}"
    exit 1
fi

echo "=============================="
echo "BES Live ISO Builder"
echo "=============================="
echo "Architecture:  $ARCH"
echo "Output:        $OUTPUT"
echo "Installer:     $INSTALLER_BIN"
echo "Cloud image:   $CLOUD_IMAGE"
echo "Suite:         $UBUNTU_SUITE"
echo "BESCONF size:  ${BESCONF_SIZE_MB} MiB"
echo "Build date:    $BUILD_DATE"
echo "=============================="
echo ""

# ============================================================
# State tracking for cleanup
# ============================================================
WORK_DIR=""
MNT_ROOTFS=""
MNT_ESP=""
CHROOT_MOUNTS_ACTIVE=0
EXTRACT_LOOP=""

cleanup() {
    local exit_code=$?
    echo ""
    if [ $exit_code -ne 0 ]; then
        echo "!!! Build failed (exit code $exit_code), cleaning up..."
    else
        echo "Cleaning up..."
    fi

    set +e

    if [ $CHROOT_MOUNTS_ACTIVE -eq 1 ] && [ -n "$MNT_ROOTFS" ]; then
        umount "$MNT_ROOTFS/dev/pts" 2>/dev/null
        umount "$MNT_ROOTFS/dev"     2>/dev/null
        umount "$MNT_ROOTFS/proc"    2>/dev/null
        umount "$MNT_ROOTFS/sys"     2>/dev/null
        umount "$MNT_ROOTFS/run"     2>/dev/null
    fi

    [ -n "$MNT_ESP" ] && mountpoint -q "$MNT_ESP" 2>/dev/null && umount "$MNT_ESP"
    [ -n "$MNT_ROOTFS" ] && mountpoint -q "$MNT_ROOTFS" 2>/dev/null && umount "$MNT_ROOTFS"
    [ -n "$EXTRACT_LOOP" ] && losetup -d "$EXTRACT_LOOP" 2>/dev/null

    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi

    if [ $exit_code -ne 0 ]; then
        rm -f "$OUTPUT"
    fi
}
trap cleanup EXIT

WORK_DIR="$(mktemp -d -t bes-iso-XXXXXX)"
MNT_ROOTFS="$WORK_DIR/rootfs"
MNT_ESP="$WORK_DIR/esp-mnt"
STAGING="$WORK_DIR/staging"

mkdir -p "$MNT_ROOTFS" "$MNT_ESP" "$STAGING"

# ============================================================
# Phase 1: Build live rootfs via debootstrap
# ============================================================
# r[impl iso.base+2]
echo "==> Phase 1: Building live rootfs (default variant)..."

DEBOOTSTRAP_EXTRA_ARGS=()
if [ ! -f /usr/share/keyrings/ubuntu-archive-keyring.gpg ]; then
    echo "    (Ubuntu keyring not found on host -- using --no-check-gpg)"
    DEBOOTSTRAP_EXTRA_ARGS+=(--no-check-gpg)
fi

debootstrap \
    --arch="$ARCH" \
    "${DEBOOTSTRAP_EXTRA_ARGS[@]}" \
    "$UBUNTU_SUITE" "$MNT_ROOTFS" "$UBUNTU_MIRROR"

# ============================================================
# Phase 2: Install packages in chroot (including live-boot)
# ============================================================
echo "==> Phase 2: Installing live environment packages..."

mount -t proc proc "$MNT_ROOTFS/proc"
mount -t sysfs sysfs "$MNT_ROOTFS/sys"
mount --bind /dev "$MNT_ROOTFS/dev"
mount --bind /dev/pts "$MNT_ROOTFS/dev/pts"
mount -t tmpfs tmpfs "$MNT_ROOTFS/run"
CHROOT_MOUNTS_ACTIVE=1

# The default debootstrap variant installs systemd-resolved, which creates
# /etc/resolv.conf as a symlink to /run/systemd/resolve/stub-resolv.conf.
# That target doesn't exist yet (no systemd running), so the symlink is
# dangling and cp refuses to write through it. Remove it first.
rm -f "$MNT_ROOTFS/etc/resolv.conf"
if [ -f /etc/resolv.conf ]; then
    cp --dereference /etc/resolv.conf "$MNT_ROOTFS/etc/resolv.conf"
elif [ -f /run/systemd/resolve/stub-resolv.conf ]; then
    cp --dereference /run/systemd/resolve/stub-resolv.conf "$MNT_ROOTFS/etc/resolv.conf"
else
    echo "nameserver 1.1.1.1" > "$MNT_ROOTFS/etc/resolv.conf"
fi

# r[impl iso.minimal+3]
# r[impl iso.live-boot]
# r[impl iso.offline]
# r[impl iso.network-tools+3]
# Enable the universe repository (live-boot is not in main)
cat > "$MNT_ROOTFS/etc/apt/sources.list.d/universe.list" << SOURCES
deb $UBUNTU_MIRROR $UBUNTU_SUITE main universe
deb $UBUNTU_MIRROR $UBUNTU_SUITE-updates main universe
deb $UBUNTU_MIRROR $UBUNTU_SUITE-security main universe
SOURCES

chroot "$MNT_ROOTFS" bash -c "
    export DEBIAN_FRONTEND=noninteractive
    export PATH='/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin'

    apt-get update -q

    apt-get install -y -q --no-install-recommends \
        linux-generic \
        linux-firmware \
        initramfs-tools \
        live-boot \
        live-boot-initramfs-tools \
        systemd-sysv \
        parted \
        gdisk \
        cloud-guest-utils \
        zstd \
        cryptsetup \
        tpm2-tools \
        btrfs-progs \
        lvm2 \
        dosfstools \
        mtools \
        pciutils \
        usbutils \
        curl

    # Install tailscale for 'tailscale netcheck' diagnostics during installation.
    # curl is installed above; tailscale needs its own apt repo.
    curl -fsSL https://pkgs.tailscale.com/stable/ubuntu/${UBUNTU_SUITE}.noarmor.gpg \
        -o /usr/share/keyrings/tailscale-archive-keyring.gpg
    curl -fsSL https://pkgs.tailscale.com/stable/ubuntu/${UBUNTU_SUITE}.tailscale-keyring.list \
        -o /etc/apt/sources.list.d/tailscale.list
    apt-get update -q
    apt-get install -y -q --no-install-recommends tailscale

    apt-get clean
    rm -rf /var/lib/apt/lists/*
"

# r[impl iso.verity.initramfs-hook]
# Install the initramfs hook that copies veritysetup into the initramfs,
# and the premount script that sets up dm-verity on the live squashfs.
# Source files live in iso/initramfs/ alongside this script.
install -D -m 755 "$SCRIPT_DIR/initramfs/hooks/verity" \
    "$MNT_ROOTFS/usr/share/initramfs-tools/hooks/verity"
install -D -m 755 "$SCRIPT_DIR/initramfs/scripts/live-premount/verity" \
    "$MNT_ROOTFS/usr/share/initramfs-tools/scripts/live-premount/verity"

# Rebuild initramfs to include the verity hook and premount script
echo "    Rebuilding initramfs to include verity hook..."
chroot "$MNT_ROOTFS" /usr/sbin/update-initramfs -u -k all

# r[impl iso.network-config+2]
# Configure netplan to DHCP on all Ethernet interfaces.
mkdir -p "$MNT_ROOTFS/etc/netplan"
cat > "$MNT_ROOTFS/etc/netplan/01-all-en-dhcp.yaml" << 'NETCFG'
network:
  version: 2
  ethernets:
    all-en:
      match:
        name: "en*"
      dhcp4: true
NETCFG
chmod 600 "$MNT_ROOTFS/etc/netplan/01-all-en-dhcp.yaml"

# live-boot's initramfs networking script (9990-networking.sh) tries to write
# /root/etc/network/interfaces during early boot. The directory doesn't exist
# because we use netplan, not ifupdown. Create it so the script doesn't print
# "can't create /root/etc/network/interfaces" on the console.
mkdir -p "$MNT_ROOTFS/etc/network"

# r[impl iso.blacklist-drm]
# The TUI installer runs on a text console and needs only the EFI framebuffer
# (efifb/simplefb). Blacklist all DRM/GPU drivers to avoid spurious errors
# (e.g. vmwgfx failing under VirtualBox) and speed up boot.
# Uses "install ... /bin/false" instead of "blacklist" to also prevent
# transitive loading by other modules.
cat > "$MNT_ROOTFS/etc/modprobe.d/blacklist-gpu.conf" << 'MODPROBE'
# BES live ISO: no GPU drivers needed for TUI installer
install vmwgfx /bin/false
install qxl /bin/false
install bochs /bin/false
install cirrus-qemu /bin/false
install vboxvideo /bin/false
install virtio-gpu /bin/false
install ast /bin/false
install mgag200 /bin/false
install hibmc-drm /bin/false
install hyperv_drm /bin/false
install nouveau /bin/false
install i915 /bin/false
install xe /bin/false
install amdgpu /bin/false
install radeon /bin/false
install drm_vram_helper /bin/false
MODPROBE

# ============================================================
# Phase 3: Install the TUI installer and configure autostart
# ============================================================
echo "==> Phase 3: Installing TUI installer binary and configuring autostart..."
install -m 755 "$INSTALLER_BIN" "$MNT_ROOTFS/usr/local/bin/bes-installer"

# Write build info file so the installer can display it
cat > "$MNT_ROOTFS/etc/bes-build-info" << BUILDINFO
BUILD_DATE=$BUILD_DATE
ARCH=$ARCH
BUILDINFO

# r[impl iso.boot.autostart+3]
# Wrapper script: runs the installer with logging to a file (not piped
# through tee, which would break the TUI's alternate screen mode).
# If the installer crashes, it leaves the alternate screen and shows
# the error on the TTY.
cat > "$MNT_ROOTFS/usr/local/bin/bes-installer-wrapper" << 'WRAPPER'
#!/bin/bash
LOG=/var/log/bes-installer.log

/usr/local/bin/bes-installer --log "$LOG"
RC=$?

if [ "$RC" -ne 0 ]; then
    # Installer crashed — make sure we're out of alternate screen mode
    printf '\033[?1049l'
    echo ""
    echo "=========================================="
    echo " BES Installer exited with error (rc=$RC)"
    echo "=========================================="
    echo ""
    if [ -f "$LOG" ]; then
        echo "Log output:"
        cat "$LOG"
        echo ""
    fi
    echo "Press Enter to retry, or Ctrl-Alt-F1 for a shell."
    read -r
fi

exit "$RC"
WRAPPER
chmod 755 "$MNT_ROOTFS/usr/local/bin/bes-installer-wrapper"

# Oneshot service to switch to tty2 early in boot, before the installer starts.
# Runs as a separate unit so it doesn't depend on the installer's TTY context.
cat > "$MNT_ROOTFS/etc/systemd/system/bes-chvt.service" << 'UNIT'
[Unit]
Description=Switch to VT2 for BES Installer
After=systemd-vconsole-setup.service
Before=bes-installer.service
DefaultDependencies=no

[Service]
Type=oneshot
ExecStart=/usr/bin/chvt 2
ExecStart=/usr/bin/dmesg -n 1
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
UNIT

cat > "$MNT_ROOTFS/etc/systemd/system/bes-installer.service" << 'UNIT'
[Unit]
Description=BES Installer TUI
After=multi-user.target systemd-logind.service bes-chvt.service
Conflicts=getty@tty2.service autovt@tty2.service

[Service]
Type=simple
ExecStart=/usr/local/bin/bes-installer-wrapper
StandardInput=tty
StandardOutput=tty
StandardError=tty
TTYPath=/dev/tty2
TTYReset=yes
TTYVHangup=no
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
UNIT

chroot "$MNT_ROOTFS" systemctl enable bes-chvt.service
chroot "$MNT_ROOTFS" systemctl enable bes-installer.service

# Disable getty and autovt on tty2 so they don't compete with the installer
chroot "$MNT_ROOTFS" systemctl mask getty@tty2.service
chroot "$MNT_ROOTFS" systemctl mask autovt@tty2.service

# Enable root autologin on tty1 so users can debug the live environment.
# Alt+F1 from the installer reaches a root shell without needing a password.
mkdir -p "$MNT_ROOTFS/etc/systemd/system/getty@tty1.service.d"
cat > "$MNT_ROOTFS/etc/systemd/system/getty@tty1.service.d/autologin.conf" << 'DROPIN'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin root --noclear %I $TERM
DROPIN

# Also allow root login with no password on other ttys (live system only)
chroot "$MNT_ROOTFS" bash -c "passwd -d root"

# Prevent systemd-logind from spawning VTs on demand for tty2
mkdir -p "$MNT_ROOTFS/etc/systemd/logind.conf.d"
cat > "$MNT_ROOTFS/etc/systemd/logind.conf.d/reserve-tty2.conf" << 'LOGIND'
[Login]
ReserveVT=2
LOGIND

# r[impl iso.config-partition]
# Mount unit for the BESCONF partition so the installer can find bes-install.toml.
# On USB boot this is a writable FAT32 partition; on optical boot it may not exist
# (FailureAction is intentionally absent so the unit failing is non-fatal).
cat > "$MNT_ROOTFS/etc/systemd/system/run-besconf.mount" << 'UNIT'
[Unit]
Description=BES Configuration Partition
After=local-fs-pre.target
DefaultDependencies=no

[Mount]
What=/dev/disk/by-label/BESCONF
Where=/run/besconf
Type=vfat
Options=ro,noatime,iocharset=ascii
TimeoutSec=5

[Install]
WantedBy=local-fs.target
UNIT

# Automount so we don't block boot if the partition is absent (optical media)
cat > "$MNT_ROOTFS/etc/systemd/system/run-besconf.automount" << 'UNIT'
[Unit]
Description=BES Configuration Partition Automount
ConditionPathExists=/dev/disk/by-label/BESCONF

[Automount]
Where=/run/besconf
TimeoutIdleSec=60

[Install]
WantedBy=local-fs.target
UNIT

chroot "$MNT_ROOTFS" systemctl enable run-besconf.automount

echo "bes-installer" > "$MNT_ROOTFS/etc/hostname"
chroot "$MNT_ROOTFS" systemd-machine-id-setup 2>/dev/null || true

# ============================================================
# Phase 4: Unmount chroot and create squashfs
# ============================================================
echo "==> Phase 4: Unmounting chroot and creating squashfs..."
umount "$MNT_ROOTFS/dev/pts"
umount "$MNT_ROOTFS/dev"
umount "$MNT_ROOTFS/proc"
umount "$MNT_ROOTFS/sys"
umount "$MNT_ROOTFS/run"
CHROOT_MOUNTS_ACTIVE=0

rm -f "$MNT_ROOTFS/etc/resolv.conf"
ln -sf /run/systemd/resolve/stub-resolv.conf "$MNT_ROOTFS/etc/resolv.conf"

rm -rf "$MNT_ROOTFS/tmp/"*
rm -rf "$MNT_ROOTFS/var/cache/apt/archives/"*.deb
rm -rf "$MNT_ROOTFS/var/lib/apt/lists/"*

# Copy vmlinuz and initrd out of rootfs BEFORE squashing
echo "    Copying kernel and initrd from rootfs..."
mkdir -p "$STAGING/live"

VMLINUZ="$(find "$MNT_ROOTFS/boot" -maxdepth 1 -name 'vmlinuz-*' -not -name '*.old' -type f | sort -V | tail -1)"
INITRD="$(find "$MNT_ROOTFS/boot" -maxdepth 1 -name 'initrd.img-*' -not -name '*.old' -type f | sort -V | tail -1)"

if [ -z "$VMLINUZ" ] || [ -z "$INITRD" ]; then
    echo "ERROR: could not find vmlinuz or initrd in rootfs /boot"
    echo "Full /boot listing:"
    find "$MNT_ROOTFS/boot" -ls 2>/dev/null || true
    exit 1
fi

cp "$VMLINUZ" "$STAGING/live/vmlinuz"
cp "$INITRD" "$STAGING/live/initrd.img"
echo "    vmlinuz: $(du -h "$STAGING/live/vmlinuz" | cut -f1)"
echo "    initrd:  $(du -h "$STAGING/live/initrd.img" | cut -f1)"

echo "    Creating squashfs (this may take a while)..."
# live-boot expects the squashfs at /live/filesystem.squashfs
mksquashfs "$MNT_ROOTFS" "$STAGING/live/filesystem.squashfs" \
    -comp xz -no-exports -noappend -quiet
rm -rf "$MNT_ROOTFS"
echo "    squashfs: $(du -h "$STAGING/live/filesystem.squashfs" | cut -f1)"

# ============================================================
# Phase 4b: Add verity to squashfs rootfs
# ============================================================
# r[impl iso.verity.squashfs]
# r[impl iso.verity.layout]
# r[impl iso.verity.build-deps]
echo "==> Phase 4b: Adding verity to squashfs rootfs..."

SQFS_HASHTREE="$WORK_DIR/filesystem.squashfs.hashtree"
SQFS_VERITY_OUTPUT="$(veritysetup format "$STAGING/live/filesystem.squashfs" "$SQFS_HASHTREE" 2>&1)"
LIVE_ROOTHASH="$(echo "$SQFS_VERITY_OUTPUT" | grep "Root hash:" | awk '{print $NF}')"
echo "    live verity root hash: $LIVE_ROOTHASH"

# Append hash tree + size trailer (self-describing verity layout)
SQFS_HASHTREE_SIZE="$(stat --format='%s' "$SQFS_HASHTREE")"
cat "$SQFS_HASHTREE" >> "$STAGING/live/filesystem.squashfs"
rm -f "$SQFS_HASHTREE"
python3 -c "import struct,sys; sys.stdout.buffer.write(struct.pack('<Q', $SQFS_HASHTREE_SIZE))" >> "$STAGING/live/filesystem.squashfs"
echo "    squashfs blob (sqfs+verity): $(du -h "$STAGING/live/filesystem.squashfs" | cut -f1)"

# ============================================================
# Phase 5: Extract partition images from cloud image
# ============================================================
# r[impl iso.contents+3]
# r[impl iso.images-partition]
echo "==> Phase 5: Extracting partition images from cloud image..."
IMAGES_STAGING="$WORK_DIR/images-staging"
mkdir -p "$IMAGES_STAGING"

CLOUD_RAW="$WORK_DIR/cloud.raw"
if [[ "$CLOUD_IMAGE" == *.zst ]]; then
    echo "    Decompressing $CLOUD_IMAGE ..."
    zstd -d "$CLOUD_IMAGE" -o "$CLOUD_RAW"
else
    echo "    Copying $CLOUD_IMAGE (already uncompressed)..."
    cp "$CLOUD_IMAGE" "$CLOUD_RAW"
fi

EXTRACT_LOOP="$(losetup -f --show -P "$CLOUD_RAW")"
echo "    Loop device: $EXTRACT_LOOP"
partprobe "$EXTRACT_LOOP"
udevadm settle
sleep 1

PART_COUNT="$(lsblk -ln -o NAME "$EXTRACT_LOOP" | grep -c "^$(basename "$EXTRACT_LOOP")p")"
if [ "$PART_COUNT" -ne 3 ]; then
    echo "ERROR: expected 3 partitions in cloud image, got $PART_COUNT"
    exit 1
fi

EFI_PART="${EXTRACT_LOOP}p1"
XBOOT_PART="${EXTRACT_LOOP}p2"
ROOT_PART="${EXTRACT_LOOP}p3"

PART_NAMES=("efi" "xboot" "root")
PART_DEVS=("$EFI_PART" "$XBOOT_PART" "$ROOT_PART")

# Read partition geometry via sfdisk JSON output
SFDISK_JSON="$(sfdisk --json "$EXTRACT_LOOP")"
SECTOR_SIZE="$(echo "$SFDISK_JSON" | jq '.partitiontable.sectorsize')"

PART_TYPES=()
PART_SIZES_MIB=()
for i in 0 1 2; do
    PART_TYPE="$(echo "$SFDISK_JSON" | jq -r ".partitiontable.partitions[$i].type")"
    PART_SIZE_SECTORS="$(echo "$SFDISK_JSON" | jq ".partitiontable.partitions[$i].size")"
    SIZE_MIB=$(( (PART_SIZE_SECTORS * SECTOR_SIZE) / 1048576 ))
    PART_TYPES+=("$PART_TYPE")
    PART_SIZES_MIB+=("$SIZE_MIB")
done

# Root partition uses size_mib=0 to mean "use all remaining space"
PART_SIZES_MIB[2]=0

for idx in 0 1 2; do
    NAME="${PART_NAMES[$idx]}"
    DEV="${PART_DEVS[$idx]}"

    echo "    Extracting $NAME partition from $DEV ..."
    dd if="$DEV" of="$IMAGES_STAGING/${NAME}.img" bs=4M status=none

    IMG_SIZE="$(stat --format='%s' "$IMAGES_STAGING/${NAME}.img")"
    echo "    ${NAME}.img: $(( IMG_SIZE / 1048576 )) MiB"
done

losetup -d "$EXTRACT_LOOP"
EXTRACT_LOOP=""
rm -f "$CLOUD_RAW"

# Generate partitions.json
echo "    Generating partitions.json ..."
jq -n \
    --arg arch "$ARCH" \
    --arg efi_type "${PART_TYPES[0]}" \
    --argjson efi_size "${PART_SIZES_MIB[0]}" \
    --arg xboot_type "${PART_TYPES[1]}" \
    --argjson xboot_size "${PART_SIZES_MIB[1]}" \
    --arg root_type "${PART_TYPES[2]}" \
    --argjson root_size "${PART_SIZES_MIB[2]}" \
    '{
        arch: $arch,
        partitions: [
            { label: "efi",   type_uuid: $efi_type,   size_mib: $efi_size,   image: "efi.img" },
            { label: "xboot", type_uuid: $xboot_type,  size_mib: $xboot_size, image: "xboot.img" },
            { label: "root",  type_uuid: $root_type,   size_mib: $root_size,  image: "root.img" }
        ]
    }' > "$IMAGES_STAGING/partitions.json"

echo "    partitions.json:"
cat "$IMAGES_STAGING/partitions.json"
echo ""

# ============================================================
# Phase 5b: Build images squashfs with verity
# ============================================================
# r[impl iso.images-partition]
# r[impl iso.verity.images]
# r[impl iso.verity.layout]
# r[impl iso.verity.build-deps]
echo "==> Phase 5b: Building images squashfs with verity..."

IMAGES_SQFS="$WORK_DIR/images.squashfs"
mksquashfs "$IMAGES_STAGING" "$IMAGES_SQFS" \
    -comp zstd -no-exports -noappend -quiet
rm -rf "$IMAGES_STAGING"
echo "    images squashfs: $(du -h "$IMAGES_SQFS" | cut -f1)"

IMAGES_HASHTREE="$WORK_DIR/images.squashfs.hashtree"
VERITY_OUTPUT="$(veritysetup format "$IMAGES_SQFS" "$IMAGES_HASHTREE" 2>&1)"
IMAGES_ROOTHASH="$(echo "$VERITY_OUTPUT" | grep "Root hash:" | awk '{print $NF}')"
echo "    images verity root hash: $IMAGES_ROOTHASH"

# Append hash tree + size trailer (self-describing verity layout)
HASHTREE_SIZE="$(stat --format='%s' "$IMAGES_HASHTREE")"
cat "$IMAGES_HASHTREE" >> "$IMAGES_SQFS"
rm -f "$IMAGES_HASHTREE"
python3 -c "import struct,sys; sys.stdout.buffer.write(struct.pack('<Q', $HASHTREE_SIZE))" >> "$IMAGES_SQFS"
echo "    images blob (sqfs+verity): $(du -h "$IMAGES_SQFS" | cut -f1)"

# ============================================================
# Phase 6: Build GRUB EFI bootloader and ESP image
# ============================================================
# r[impl iso.boot.uefi]
echo "==> Phase 6: Building EFI boot image..."

# Place EFI/BOOT inside the ISO tree so xorriso can see it and so that
# firmware/tools that look for the EFI directory in the ISO filesystem
# find it there.
mkdir -p "$STAGING/EFI/BOOT"
mkdir -p "$STAGING/boot/grub"

grub-mkimage \
    -o "$STAGING/EFI/BOOT/$GRUB_EFI_NAME" \
    -O "$GRUB_TARGET" \
    -p /boot/grub \
    part_gpt part_msdos fat iso9660 normal boot linux configfile loopback \
    search search_label search_fs_uuid search_fs_file ls cat echo test true \
    chain efinet

cat > "$STAGING/boot/grub/grub.cfg" << GRUBCFG
set timeout=1
set default=0

insmod all_video

search --file --no-floppy --set=root /live/vmlinuz

menuentry "BES Installer (${ARCH}, built ${BUILD_DATE})" {
    linux /live/vmlinuz boot=live toram console=tty1 live.verity.roothash=${LIVE_ROOTHASH} images.verity.roothash=${IMAGES_ROOTHASH}
    initrd /live/initrd.img
}

menuentry "BES Installer (${ARCH}, built ${BUILD_DATE}) -- quiet" {
    linux /live/vmlinuz boot=live toram quiet console=tty1 live.verity.roothash=${LIVE_ROOTHASH} images.verity.roothash=${IMAGES_ROOTHASH}
    initrd /live/initrd.img
}
GRUBCFG

# Build a FAT32 image for the El Torito EFI boot catalog entry.
# This image is embedded inside the ISO filesystem at /boot/efi.img and
# is also exposed as a GPT ESP via --efi-boot-part for USB boot.
ESP_IMG="$STAGING/boot/efi.img"
ESP_SIZE_MB=16

truncate -s "${ESP_SIZE_MB}M" "$ESP_IMG"
mkfs.vfat -F 12 -n ESP "$ESP_IMG" >/dev/null

mount -o loop "$ESP_IMG" "$MNT_ESP"
mkdir -p "$MNT_ESP/EFI/BOOT"
mkdir -p "$MNT_ESP/boot/grub"
cp "$STAGING/EFI/BOOT/$GRUB_EFI_NAME" "$MNT_ESP/EFI/BOOT/$GRUB_EFI_NAME"
cp "$STAGING/boot/grub/grub.cfg" "$MNT_ESP/boot/grub/grub.cfg"
umount "$MNT_ESP"

echo "    EFI image: $(du -h "$ESP_IMG" | cut -f1)"
echo "    GRUB target: $GRUB_TARGET ($GRUB_EFI_NAME)"

# ============================================================
# Phase 7: Build BESCONF FAT32 partition image
# ============================================================
# r[impl iso.config-partition]
echo "==> Phase 7: Building BESCONF partition image..."

BESCONF_IMG="$WORK_DIR/besconf.img"
truncate -s "${BESCONF_SIZE_MB}M" "$BESCONF_IMG"
mkfs.vfat -F 12 -n BESCONF "$BESCONF_IMG" >/dev/null

# Write a template config file with all options commented out
mount -o loop "$BESCONF_IMG" "$MNT_ESP"
cat > "$MNT_ESP/bes-install.toml" << 'TEMPLATE'
# BES Installer Configuration
#
# Uncomment and edit the options below to pre-configure the installer.
# When written to the BESCONF partition of a USB stick, these values
# will be used as defaults (or to drive a fully automatic install).

# Run fully automatically without prompts.
# Requires at minimum: disk-encryption and disk.
# auto = true

# Disk encryption mode: "tpm", "keyfile", or "none".
#   tpm     - LUKS + TPM PCR 1 (requires a TPM; default when TPM present)
#   keyfile - LUKS + keyfile on boot partition (default when no TPM)
#   none    - No encryption (cloud image)
# disk-encryption = "tpm"

# Target disk: a device path (e.g. "/dev/sda") or a selection strategy.
# Strategies:
#   "largest-ssd" - largest SSD by capacity
#   "largest"     - largest disk of any type
#   "smallest"    - smallest disk of any type
# disk = "largest-ssd"

# hostname = "server-01"
# tailscale-authkey = "tskey-auth-xxxxx"
# ssh-authorized-keys = [
#   "ssh-ed25519 AAAA... admin@example.com",
# ]

# Pre-determined recovery passphrase for disk encryption.
# Must be at least 25 characters, printable ASCII only (no whitespace).
# If not set, a random diceware passphrase is generated.
# recovery-passphrase = "xK9$mP2vL!nQ7wR4jH6dT0yB3fA8s"

# Save recovery keys to recovery-keys.txt on the BESCONF partition.
# Each line contains: passphrase, root partition UUID, machine serial.
# Only works when BESCONF is writable (USB boot, not optical).
# save-recovery-keys = false
TEMPLATE
umount "$MNT_ESP"

echo "    BESCONF image: $(du -h "$BESCONF_IMG" | cut -f1)"

# ============================================================
# Phase 8: Produce hybrid ISO with xorriso
# ============================================================
# r[impl iso.format]
# r[impl iso.hybrid]
# r[impl iso.usb]
echo "==> Phase 8: Producing hybrid ISO9660 image with xorriso..."

mkdir -p "$(dirname "$OUTPUT")"

xorriso -as mkisofs \
    -o "$OUTPUT" \
    -V "BES_INSTALLER" \
    -R -J \
    -iso-level 3 \
    \
    -e boot/efi.img \
    -no-emul-boot \
    \
    --efi-boot-part --efi-boot-image \
    \
    -append_partition 3 EBD0A0A2-B9E5-4433-87C0-68B6B72699C7 "$BESCONF_IMG" \
    \
    -append_partition 4 0FC63DAF-8483-4772-8E79-3D69D8477DE4 "$IMAGES_SQFS" \
    \
    "$STAGING"

# Clean up working directory
rm -rf "$WORK_DIR"
WORK_DIR=""

trap - EXIT

echo ""
echo "=============================="
echo "Live ISO built successfully"
echo "=============================="
echo "Output: $OUTPUT"
echo "Size:   $(du -h "$OUTPUT" | cut -f1)"
echo "SHA256: $(sha256sum "$OUTPUT" | cut -d' ' -f1)"
echo ""
echo "Boot in a VM:"
echo "  Attach $OUTPUT as a CD/DVD drive (UEFI mode)"
echo ""
echo "Write to USB:"
echo "  sudo dd if=$OUTPUT of=/dev/sdX bs=4M status=progress"
echo ""
echo "To pre-configure on USB, mount the BESCONF partition and place bes-install.toml:"
echo "  lsblk -o NAME,LABEL /dev/sdX   # find the BESCONF partition"
echo "  mount /dev/sdXN /mnt && cp bes-install.toml /mnt/ && umount /mnt"
echo "=============================="
