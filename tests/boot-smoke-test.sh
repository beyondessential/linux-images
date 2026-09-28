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
# neither shows up. This is a human-readable copy of the output only: a
# getty starts on this same device once the guest reaches a login prompt
# and hangs up whoever had it open, silently discarding anything still
# being written to it — so the pass/fail verdict is never read from here.
SERIAL_DEV=/dev/ttyS0
[ -e "$SERIAL_DEV" ] || SERIAL_DEV=/dev/ttyAMA0
[ -e "$SERIAL_DEV" ] || SERIAL_DEV=/dev/console

# The dedicated result channel: a virtio-serial port no getty ever touches,
# so the verdict reaches the host even after the console above is revoked.
# It's created by udev shortly after the virtio_console driver probes, so
# wait briefly rather than racing it; fall back to the console alone if it
# never shows up (e.g. run without the matching QEMU device).
RESULTS_DEV=/dev/virtio-ports/bes.test-results
for _ in $(seq 1 20); do
  [ -e "$RESULTS_DEV" ] && break
  sleep 0.25
done

if [ -e "$RESULTS_DEV" ]; then
  exec > >(tee "$RESULTS_DEV" "$SERIAL_DEV") 2>&1
else
  exec > "$SERIAL_DEV" 2>&1
  echo "WARNING: $RESULTS_DEV not present; results are only on the serial console"
fi

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

ROOT_PART="$(readlink -f /dev/disk/by-partlabel/root)"
ROOT_NAME="${ROOT_PART##*/}"
DISK_NAME="$(lsblk -no PKNAME "$ROOT_PART")"
DISK="/dev/$DISK_NAME"

# The root partition's end, as a distance in 512-byte sectors from the end of
# the disk.
root_partition_slack() {
  local disk_size start size
  disk_size="$(cat "/sys/block/$DISK_NAME/size")"
  start="$(cat "/sys/block/$DISK_NAME/$ROOT_NAME/start")"
  size="$(cat "/sys/block/$DISK_NAME/$ROOT_NAME/size")"
  echo $((disk_size - start - size))
}

# r[verify image.growth.service+4]
# The unit is a oneshot that remains active once its run has succeeded.
check "grow-root-filesystem ran" \
  test "$(systemctl show -p ActiveState --value grow-root-filesystem.service)" = active
check "grow-root-filesystem succeeded" \
  test "$(systemctl show -p Result --value grow-root-filesystem.service)" = success
# test-boot enlarges the disk before booting; allow for the backup GPT and
# partition alignment.
check "root partition was grown to the end of the disk" \
  test "$(root_partition_slack)" -lt 4096

# r[verify image.btrfs.format+2]
check "root is btrfs" stat -f -c%T /
# r[verify image.btrfs.compression]
check "compression active in /proc/mounts" grep -q 'compress=' /proc/mounts

# r[verify image.variant.types+4]
VARIANT=$(cat /etc/bes/image-variant 2>/dev/null || echo "unknown")
echo "Variant: $VARIANT"
case "$VARIANT" in
  metal | cloud | pi | luks-tpm | luks-keyfile | plain) VARIANT_DOCUMENTED=yes ;;
  *) VARIANT_DOCUMENTED=no ;;
esac
check "image-variant is a documented value" test "$VARIANT_DOCUMENTED" = yes

ROOT_IS_LUKS=no
cryptsetup isLuks "$ROOT_PART" 2>/dev/null && ROOT_IS_LUKS=yes
LUKS_ACTIVE=no
[ "$(lsblk -no TYPE /dev/mapper/root 2>/dev/null)" = crypt ] && LUKS_ACTIVE=yes
check "LUKS is active exactly when the root partition holds a LUKS volume" \
  test "$ROOT_IS_LUKS" = "$LUKS_ACTIVE"
# The spec has runtime code detect LUKS rather than infer it from this file,
# so only the build-time values, which each fix the volume type, are held to
# one.
case "$VARIANT" in
  metal | pi)
    # r[verify image.luks.format]
    check "root partition holds a LUKS volume on $VARIANT" test "$ROOT_IS_LUKS" = yes
    ;;
  cloud)
    check "root partition holds no LUKS volume on cloud" test "$ROOT_IS_LUKS" = no
    ;;
esac

# r[verify image.credentials.ubuntu-user]
check "ubuntu user exists" id ubuntu
# r[verify image.base.machine-id+2]
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

# Prints every volume-key digest in the LUKS2 header on $1, one per line,
# as lowercase hex. Read from the header's JSON metadata rather than through
# the text reader the record was made with, so a mismatch between that reader
# and the header's layout cannot pass for a re-encryption.
luks_all_digests() {
  local b64
  cryptsetup luksDump --dump-json-metadata "$1" | jq -r '.digests[].digest' |
    while IFS= read -r b64; do
      printf '%s' "$b64" | base64 -d | od -An -v -tx1 | tr -d ' \n'
      echo
    done
}

# Re-encryption under a fresh master key replaces the volume-key digest, and
# may renumber it: the header must carry at least one well-formed digest and
# none of them may be the recorded one.
luks_digest_rotated() {
  local digests d
  [[ "${LUKS_DIGEST:-}" =~ ^[0-9a-f]{64}$ ]] || return 1
  digests="$(luks_all_digests "$1")" || return 1
  [ -n "$digests" ] || return 1
  while IFS= read -r d; do
    [[ "$d" =~ ^([0-9a-f]{2}){16,}$ ]] || return 1
    [ "$d" != "${LUKS_DIGEST,,}" ] || return 1
  done <<<"$digests"
}

if [ -f /etc/bes/build-identity ] && [ -r /root/bes-identity-lib.sh ]; then
  # shellcheck source=/dev/null
  . /root/bes-identity-lib.sh
  # shellcheck disable=SC1091 # generated record, shell-sourceable KEY=value lines
  . /etc/bes/build-identity

  XBOOT_PART="$(readlink -f /dev/disk/by-partlabel/xboot)"
  if [ "$VARIANT" = "pi" ]; then
    BOOT1_PART="$(readlink -f /dev/disk/by-partlabel/firmware)"
  else
    BOOT1_PART="$(readlink -f /dev/disk/by-partlabel/efi)"
  fi

  if [ "$ROOT_IS_LUKS" = yes ]; then
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

  if [ "$ROOT_IS_LUKS" = yes ]; then
    # r[verify image.identity.rotate]
    check "LUKS_UUID rotated on first boot" rotated LUKS_UUID
    # r[verify image.identity.luks-rekey]
    check "LUKS volume key replaced on first boot" luks_digest_rotated "$ROOT_PART"
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

sync
sleep 2
poweroff
