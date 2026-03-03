#!/bin/bash
# r[impl iso.format]
# r[impl iso.hybrid]
# r[impl iso.base]
# r[impl iso.live-boot]
# r[impl iso.minimal]
# r[impl iso.offline]
# r[impl iso.contents]
# r[impl iso.boot.uefi]
# r[impl iso.boot.autostart]
# r[impl iso.config-partition]
# r[impl iso.per-arch]
# r[impl iso.usb]
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
#     IMAGE_DIR       - directory containing .raw.zst images to embed
#     UBUNTU_SUITE    - Ubuntu suite name (default: noble)
#     UBUNTU_MIRROR   - mirror URL (auto-selected per arch if unset)
#     BESCONF_SIZE_MB - BESCONF partition size in MiB (default: 4)
set -euo pipefail

ARCH="${ARCH:-amd64}"
UBUNTU_SUITE="${UBUNTU_SUITE:-noble}"
BESCONF_SIZE_MB="${BESCONF_SIZE_MB:-4}"
BUILD_DATE="$(date -u +%Y-%m-%d)"
INSTALLER_BIN="${INSTALLER_BIN:?INSTALLER_BIN must point to the bes-installer binary}"
IMAGE_DIR="${IMAGE_DIR:?IMAGE_DIR must point to directory with .raw.zst images}"
OUTPUT="${OUTPUT:-output/${ARCH}/bes-installer-${ARCH}.iso}"

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

if [ ! -d "$IMAGE_DIR" ]; then
    echo "ERROR: image directory not found: $IMAGE_DIR"
    exit 1
fi

IMAGE_FILES=()
while IFS= read -r -d '' f; do
    IMAGE_FILES+=("$f")
done < <(find "$IMAGE_DIR" -maxdepth 1 -name "*.raw.zst" -print0)

if [ "${#IMAGE_FILES[@]}" -eq 0 ]; then
    echo "ERROR: no .raw.zst images found in $IMAGE_DIR"
    exit 1
fi

MISSING=()
for cmd in debootstrap mksquashfs sgdisk mkfs.vfat losetup grub-mkimage xorriso; do
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
echo "Image dir:     $IMAGE_DIR"
echo "Images:        ${IMAGE_FILES[*]}"
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
# Phase 1: Build minimal live rootfs via debootstrap
# ============================================================
echo "==> Phase 1: Building minimal live rootfs..."

DEBOOTSTRAP_EXTRA_ARGS=()
if [ ! -f /usr/share/keyrings/ubuntu-archive-keyring.gpg ]; then
    echo "    (Ubuntu keyring not found on host -- using --no-check-gpg)"
    DEBOOTSTRAP_EXTRA_ARGS+=(--no-check-gpg)
fi

debootstrap \
    --arch="$ARCH" \
    --variant=minbase \
    --include=ca-certificates \
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

if [ -f /etc/resolv.conf ]; then
    cp --dereference /etc/resolv.conf "$MNT_ROOTFS/etc/resolv.conf"
elif [ -f /run/systemd/resolve/stub-resolv.conf ]; then
    cp --dereference /run/systemd/resolve/stub-resolv.conf "$MNT_ROOTFS/etc/resolv.conf"
else
    echo "nameserver 1.1.1.1" > "$MNT_ROOTFS/etc/resolv.conf"
fi

# r[impl iso.minimal]
# r[impl iso.live-boot]
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
        initramfs-tools \
        live-boot \
        live-boot-initramfs-tools \
        systemd \
        systemd-sysv \
        dbus \
        udev \
        util-linux \
        parted \
        gdisk \
        zstd \
        cryptsetup \
        btrfs-progs \
        lvm2 \
        dosfstools \
        e2fsprogs \
        pciutils \
        usbutils \
        less

    apt-get clean
    rm -rf /var/lib/apt/lists/*
"

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

# r[impl iso.boot.autostart]
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
echo "nameserver 1.1.1.1" > "$MNT_ROOTFS/etc/resolv.conf"

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
# Phase 5: Copy disk images into staging
# ============================================================
# r[impl iso.contents]
echo "==> Phase 5: Copying disk images into staging..."
mkdir -p "$STAGING/images"
for img in "${IMAGE_FILES[@]}"; do
    echo "    $(basename "$img")"
    cp "$img" "$STAGING/images/"
done

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

cat > "$STAGING/boot/grub/grub.cfg" << 'GRUBCFG'
set timeout=3
set default=0

insmod all_video

search --file --no-floppy --set=root /live/vmlinuz

menuentry "BES Installer (__ARCH__, built __BUILD_DATE__)" {
    linux /live/vmlinuz boot=live toram quiet console=tty1
    initrd /live/initrd.img
}

menuentry "BES Installer (__ARCH__, built __BUILD_DATE__) -- verbose" {
    linux /live/vmlinuz boot=live toram console=tty1
    initrd /live/initrd.img
}
GRUBCFG

sed -i "s/__ARCH__/${ARCH}/g; s/__BUILD_DATE__/${BUILD_DATE}/g" "$STAGING/boot/grub/grub.cfg"

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
# Requires at minimum: variant and disk.
# auto = true

# Image variant: "metal" or "cloud"
#   metal - Full-disk encryption (LUKS2) with optional TPM auto-unlock
#   cloud - No encryption, for cloud VMs or environments with
#           host-level disk encryption
# variant = "metal"

# Target disk: a device path (e.g. "/dev/sda") or a selection strategy.
# Strategies:
#   "largest-ssd" - largest SSD by capacity
#   "largest"     - largest disk of any type
#   "smallest"    - smallest disk of any type
# disk = "largest-ssd"

# Disable TPM auto-enrollment (metal variant only).
# When true, the LUKS volume will not be bound to the TPM on first boot.
# disable-tpm = false

# [firstboot]
# hostname = "server-01"
# tailscale-authkey = "tskey-auth-xxxxx"
# ssh-authorized-keys = [
#   "ssh-ed25519 AAAA... admin@example.com",
# ]
TEMPLATE
umount "$MNT_ESP"

echo "    BESCONF image: $(du -h "$BESCONF_IMG" | cut -f1)"

# ============================================================
# Phase 8: Produce hybrid ISO with xorriso
# ============================================================
# r[impl iso.format]
# r[impl iso.hybrid]
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
