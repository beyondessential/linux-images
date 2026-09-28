#!/bin/bash
# In-guest boot smoke test, run once by cloud-init after first boot.
#
# Shipped into the test guest by the justfile's `_make-test-cloud-init`
# recipe (base64-encoded into a cloud-init `write_files` entry) and
# invoked via a `runcmd` list-form entry: `[bash,
# /root/boot-smoke-test.sh]`. cloud-init's runcmd module always wraps
# every runcmd entry into one /bin/sh script regardless of an embedded
# shebang, but a list-form entry's argv is executed directly rather than
# interpreted inline — here that line is `bash /root/boot-smoke-test.sh`,
# so this script still runs under bash (and its array syntax works)
# even though the wrapping script itself is sh.

# QEMU's aarch64 `virt` machine exposes its UART as ttyAMA0 (PL011), not
# ttyS0 (8250/16550, x86-only); fall back to the console device if
# neither shows up.
SERIAL_DEV=/dev/ttyS0
[ -e "$SERIAL_DEV" ] || SERIAL_DEV=/dev/ttyAMA0
[ -e "$SERIAL_DEV" ] || SERIAL_DEV=/dev/console
exec > "$SERIAL_DEV" 2>&1

PASS=0
FAIL=0
ERRORS=()

check() {
  local desc="$1"; shift
  if "$@" >/dev/null 2>&1; then
    echo "PASS: $desc"
    ((PASS++))
  else
    echo "FAIL: $desc"
    ERRORS+=("$desc")
    ((FAIL++))
  fi
}

echo "=== BES Boot Smoke Test ==="
echo ""

check "systemd reached multi-user.target" systemctl is-active multi-user.target
FAILED_UNITS=$(systemctl --failed --no-legend --no-pager | wc -l)
check "no failed systemd units" test "$FAILED_UNITS" -eq 0

# r[verify image.credentials.ssh-password-auth]
check "sshd is active" systemctl is-active ssh
# r[verify image.firewall.enabled]
check "ufw is active" systemctl is-active ufw
# r[verify image.tailscale.service-enabled]
check "tailscaled is active" systemctl is-active tailscaled
# r[verify image.growth.service+4]
check "grow-root-filesystem ran" systemctl show -p ActiveState grow-root-filesystem.service | grep -q inactive

# r[verify image.btrfs.format+2]
check "root is btrfs" stat -f -c%T /
# r[verify image.btrfs.compression]
check "compression active in /proc/mounts" grep -q 'compress=' /proc/mounts

# r[verify image.variant.types+4]
VARIANT=$(cat /etc/bes/image-variant 2>/dev/null || echo "unknown")
echo "Variant: $VARIANT"

if [ "$VARIANT" = "metal" ] || [ "$VARIANT" = "pi" ]; then
  # r[verify image.luks.format]
  check "LUKS volume is active" test -e /dev/mapper/root
fi

# r[verify image.credentials.ubuntu-user]
check "ubuntu user exists" id ubuntu
# r[verify image.base.machine-id]
check "machine-id is non-empty" test -s /etc/machine-id

# r[verify image.partition.xboot]
check "/boot is mounted" mountpoint -q /boot
# r[verify image.partition.efi] r[verify image.partition.pi-firmware]
if [ "$VARIANT" = "pi" ]; then
  check "/boot/firmware is mounted" mountpoint -q /boot/firmware
else
  check "/boot/efi is mounted" mountpoint -q /boot/efi
fi

# ------------------------------------------------------------------
# First-boot identity rotation
#
# /etc/bes/build-identity is never rewritten by the rotation module, so
# it is still the build-time baseline to compare against. The reader
# functions come from bes-identity-lib.sh itself (shipped in alongside
# this script) rather than being re-derived here: dracut only installs
# that library into the initramfs, which is discarded after
# switch-root, so it is not on the booted rootfs to source directly.
# ------------------------------------------------------------------

rotated() {
  local key="$1" rec curname cur
  rec="${!key:-}"
  curname="CUR_$key"
  cur="${!curname:-}"
  [ -n "$rec" ] && [ -n "$cur" ] && [ "${rec,,}" != "${cur,,}" ]
}

not_referenced() {
  local value="$1" file="$2"
  ! grep -qiF -- "$value" "$file"
}

luks_digest_rotated() {
  # Re-encrypting under a fresh master key can renumber digest 0 away
  # entirely, so its absence counts as rotated evidence too.
  [ -z "${CUR_LUKS_DIGEST:-}" ] || [ "${LUKS_DIGEST,,}" != "${CUR_LUKS_DIGEST,,}" ]
}

if [ -f /etc/bes/build-identity ] && [ -r /root/bes-identity-lib.sh ]; then
  # shellcheck source=/dev/null
  . /root/bes-identity-lib.sh
  # shellcheck disable=SC1091 # generated record, shell-sourceable KEY=value lines
  . /etc/bes/build-identity

  ROOT_PART="$(readlink -f /dev/disk/by-partlabel/root)"
  XBOOT_PART="$(readlink -f /dev/disk/by-partlabel/xboot)"
  if [ "$VARIANT" = "pi" ]; then
    BOOT1_PART="$(readlink -f /dev/disk/by-partlabel/firmware)"
  else
    BOOT1_PART="$(readlink -f /dev/disk/by-partlabel/efi)"
  fi
  DISK="/dev/$(lsblk -no PKNAME "$ROOT_PART")"

  ENCRYPTED=0
  if cryptsetup isLuks "$ROOT_PART" 2>/dev/null; then
    ENCRYPTED=1
    BTRFS_DEV=/dev/mapper/root
  else
    BTRFS_DEV="$ROOT_PART"
  fi

  while IFS= read -r line; do
    printf -v "CUR_${line%%=*}" '%s' "${line#*=}"
  done < <(identity_read "$DISK" "$BOOT1_PART" "$XBOOT_PART" "$ROOT_PART" "$BTRFS_DEV")

  # GRUB embeds the xboot ext4 UUID in a way that cannot be rewritten
  # before the root filesystem is mounted, so it is exempt from rotation
  # on GRUB variants (metal, cloud) — see r[image.identity.rotate]. Pi
  # has no GRUB, so its xboot UUID rotates like every other identifier.
  # This mirrors the same signal the rotation module itself uses
  # (whether xboot carries a grub.cfg) rather than hardcoding variant
  # names.
  if [ -f /boot/grub/grub.cfg ]; then
    GRUB_VARIANT=1
  else
    GRUB_VARIANT=0
  fi

  # r[verify image.identity.rotate]
  for key in DISK_GUID PARTUUID_1 PARTUUID_2 PARTUUID_3 BOOT1_SERIAL BTRFS_UUID; do
    check "$key rotated on first boot" rotated "$key"
  done
  if [ "$GRUB_VARIANT" -eq 1 ]; then
    check "XBOOT_UUID kept unchanged on GRUB variant" \
      test "${XBOOT_UUID,,}" = "${CUR_XBOOT_UUID,,}"
  else
    check "XBOOT_UUID rotated on first boot" rotated XBOOT_UUID
  fi

  if [ "$ENCRYPTED" -eq 1 ]; then
    # r[verify image.identity.rotate]
    check "LUKS_UUID rotated on first boot" rotated LUKS_UUID
    # r[verify image.identity.luks-rekey]
    check "LUKS_DIGEST rotated (or now absent) on first boot" luks_digest_rotated
  fi

  # r[verify image.identity.grub-repair] r[verify image.boot.grub-uuids]
  # grub.cfg must have been rewritten to the post-rotation values, with
  # no leftover reference to a build-time value that has since rotated
  # — keeps the pre-boot image.boot.grub-uuids check's intent holding
  # after first boot too.
  if [ "$GRUB_VARIANT" -eq 1 ] && [ -f /boot/grub/grub.cfg ]; then
    GRUB_CFG=/boot/grub/grub.cfg
    for key in DISK_GUID PARTUUID_1 PARTUUID_2 PARTUUID_3 BOOT1_SERIAL BTRFS_UUID LUKS_UUID; do
      rec="${!key:-}"
      [ -n "$rec" ] || continue
      if rotated "$key"; then
        check "grub.cfg no longer references rotated $key" not_referenced "$rec" "$GRUB_CFG"
      fi
    done
    check "grub.cfg references the current root UUID" \
      grep -qiF -- "$CUR_BTRFS_UUID" "$GRUB_CFG"
    check "grub.cfg still references the unchanged xboot UUID" \
      grep -qiF -- "$CUR_XBOOT_UUID" "$GRUB_CFG"
  fi
else
  echo "FAIL: build-identity record or identity library not available for rotation checks"
  ERRORS+=("build-identity record or identity library not available for rotation checks")
  FAIL=$((FAIL + 1))
fi

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
