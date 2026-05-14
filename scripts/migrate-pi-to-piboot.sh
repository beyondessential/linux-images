#!/bin/bash
# Migrate a pi image already flashed with the legacy single-directory
# /boot/firmware layout (kernel/initrd/DTB at the root) to the A/B
# tryboot layout under current/ + new/ + old/. See
# r[image.boot.pi-tryboot-rollback] in docs/spec/disk-images.md.
#
# Steps:
#   1. Pre-flight: this is a pi image, /boot/firmware is mounted, the
#      legacy hand-rolled helper is present (i.e. needs migration), and
#      the EEPROM firmware is recent enough to honour tryboot.
#   2. Install flash-kernel-piboot (pulls in piboot-try).
#   3. Remove the legacy helper + kernel postinst hook.
#   4. Run flash-kernel — its migrate() function copies the existing
#      kernel/initrd/DTB/overlays into current/, rewrites config.txt
#      with os_prefix= keys, and writes autoboot.txt with tryboot_a_b=1.
#   5. Verify the new layout is in place.
#
# Run as root. Re-runnable: if flash-kernel-piboot is already installed
# and the layout is already A/B, the script reports "nothing to do" and
# exits 0.
set -euo pipefail

# --- Pre-flight ---------------------------------------------------------

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root" >&2
    exit 1
fi

if [ ! -r /etc/bes/image-variant ] || [ "$(cat /etc/bes/image-variant)" != "pi" ]; then
    echo "ERROR: not a pi image (check /etc/bes/image-variant)" >&2
    exit 1
fi

if ! mountpoint -q /boot/firmware; then
    echo "ERROR: /boot/firmware is not mounted" >&2
    exit 1
fi

# Detect whether we have anything to do. The pi-try migration is
# already complete when current/state exists OR config.txt already has
# os_prefix= (mirrors needs_migrate() in /usr/share/flash-kernel/functions-piboot).
ALREADY_MIGRATED=0
if [ -f /boot/firmware/current/state ] || \
   grep -q '^os_prefix=' /boot/firmware/config.txt 2>/dev/null; then
    ALREADY_MIGRATED=1
fi

HAS_LEGACY_HELPER=0
if [ -x /usr/local/sbin/bes-pi-firmware-update ] || \
   [ -e /etc/kernel/postinst.d/zz-bes-pi-firmware ]; then
    HAS_LEGACY_HELPER=1
fi

if [ "$ALREADY_MIGRATED" -eq 1 ] && [ "$HAS_LEGACY_HELPER" -eq 0 ]; then
    echo "Already migrated (config.txt has os_prefix= and no legacy helper present). Nothing to do."
    exit 0
fi

# EEPROM firmware floor: tryboot on Pi 5 / 500 / CM5 needs firmware
# dated 2025-02-11 or later (Ubuntu 26.04 release notes).
EEPROM_FLOOR="2025-02-11"

if ! command -v vcgencmd >/dev/null 2>&1; then
    echo "ERROR: vcgencmd not found — cannot verify EEPROM firmware date" >&2
    echo "  apt-get install libraspberrypi-bin" >&2
    exit 1
fi

# `vcgencmd bootloader_version` first line is the date — either
# "YYYY-MM-DD HH:MM:SS" or "YYYY/MM/DD HH:MM:SS" depending on the EEPROM
# release. Normalise to dash form so the string comparison below works
# byte-for-byte against EEPROM_FLOOR.
EEPROM_LINE="$(vcgencmd bootloader_version | head -n1 | tr -d '\r')"
EEPROM_DATE="${EEPROM_LINE%% *}"
EEPROM_DATE="${EEPROM_DATE//\//-}"
if [ -z "$EEPROM_DATE" ] || ! [[ "$EEPROM_DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    echo "ERROR: could not parse EEPROM date from vcgencmd output: $EEPROM_LINE" >&2
    exit 1
fi

if [ "$EEPROM_DATE" \< "$EEPROM_FLOOR" ]; then
    echo "ERROR: EEPROM firmware date $EEPROM_DATE is older than $EEPROM_FLOOR" >&2
    echo "  The tryboot A/B mechanism requires newer EEPROM firmware." >&2
    echo "  Update with: sudo rpi-eeprom-update -a && sudo reboot" >&2
    echo "  Then re-run this script." >&2
    exit 1
fi
echo "EEPROM firmware date $EEPROM_DATE meets floor $EEPROM_FLOOR."

# --- Install flash-kernel-piboot ----------------------------------------

if ! dpkg -s flash-kernel-piboot >/dev/null 2>&1; then
    echo "Installing flash-kernel-piboot..."
    apt-get update -q
    # --no-install-recommends matches the image build choice (see
    # image/packages.sh + image/configure.sh).
    DEBIAN_FRONTEND=noninteractive apt-get install -y -q \
        --no-install-recommends flash-kernel-piboot
else
    echo "flash-kernel-piboot already installed."
fi

# --- Remove the legacy helper -------------------------------------------

# Order matters: drop the kernel postinst hook first so a parallel
# upgrade can't fire it after flash-kernel takes over.
if [ -e /etc/kernel/postinst.d/zz-bes-pi-firmware ]; then
    echo "Removing /etc/kernel/postinst.d/zz-bes-pi-firmware..."
    rm -f /etc/kernel/postinst.d/zz-bes-pi-firmware
fi
if [ -e /usr/local/sbin/bes-pi-firmware-update ]; then
    echo "Removing /usr/local/sbin/bes-pi-firmware-update..."
    rm -f /usr/local/sbin/bes-pi-firmware-update
fi

# --- Run flash-kernel ---------------------------------------------------
#
# flash-kernel detects Method: pi-try for the Pi 5B, sees that
# needs_migrate() is true (no current/state, no os_prefix= in
# config.txt), and runs migrate() to convert the layout in-place.
# Subsequent runs become a normal "stage assets into new/" operation.
echo "Running flash-kernel to migrate /boot/firmware layout..."
flash-kernel

# --- Verify -------------------------------------------------------------

FAIL=0
verify() {
    local desc="$1"; shift
    if "$@"; then
        echo "  OK: $desc"
    else
        echo "  FAIL: $desc" >&2
        FAIL=1
    fi
}

verify "/boot/firmware/current/ exists" test -d /boot/firmware/current
verify "/boot/firmware/current/state is good" sh -c '[ "$(cat /boot/firmware/current/state 2>/dev/null)" = good ]'
verify "/boot/firmware/current/vmlinuz exists" test -f /boot/firmware/current/vmlinuz
verify "/boot/firmware/current/initrd.img exists" test -f /boot/firmware/current/initrd.img
verify "config.txt sets os_prefix=current/" grep -q '^os_prefix=current/' /boot/firmware/config.txt
verify "config.txt sets os_prefix=new/" grep -q '^os_prefix=new/' /boot/firmware/config.txt
verify "autoboot.txt enables tryboot_a_b" grep -q '^tryboot_a_b=1' /boot/firmware/autoboot.txt
verify "legacy bes-pi-firmware-update absent" sh -c '! test -e /usr/local/sbin/bes-pi-firmware-update'
verify "legacy zz-bes-pi-firmware hook absent" sh -c '! test -e /etc/kernel/postinst.d/zz-bes-pi-firmware'

if [ "$FAIL" -ne 0 ]; then
    echo "ERROR: migration verification failed — inspect /boot/firmware" >&2
    exit 1
fi

echo ""
echo "Migration complete. Reboot to start booting from /boot/firmware/current/."
echo "Note: the first kernel apt upgrade after this point will stage assets"
echo "into /boot/firmware/new/ and trigger a one-time tryboot on reboot."
