#!/bin/bash
# First-boot rotation of the identifiers every copy of a flashed image shares.
# Runs in the initramfs as two units, before the root filesystem is mounted:
#
#   bes-identity pre   before the root volume is unlocked: GPT disk and
#                      partition GUIDs, then LUKS re-encryption and UUID.
#   bes-identity post  once the root volume is unlocked: btrfs filesystem ID,
#                      boot1 FAT serial, xboot UUID (no GRUB only), then the
#                      grub.cfg repair and, if needed, a reboot.
#
# Every step compares the identifier's current value with the build-time
# record and acts only while they are still equal (or, for re-encryption,
# while one is in progress), so an interrupted run is picked up by the next
# boot and a system whose identifiers already differ is left alone.
#
# A step that fails is logged and the boot carries on: a failed rotation
# leaves a working system with a still-shared identifier, where a halted
# boot leaves a remote device dead. The one exception is a re-encryption
# that stops part-way: that exits non-zero so the volume is not unlocked
# until a later boot has finished it.

set -u

RECORD=/etc/bes/build-identity
STATE_DIR=/run/bes-identity
XBOOT_MNT="$STATE_DIR/xboot"
KEYFILE="$STATE_DIR/empty-key"
MAPPER_DEV=/dev/mapper/root
ROOT_LINK=/dev/disk/by-partlabel/root

# shellcheck source=image/files/dracut/modules.d/90bes-identity/bes-identity-lib.sh
. /usr/lib/bes-identity/bes-identity-lib.sh

# Messages go to the console and the journal; the <N> prefix sets the
# journal priority.
log() { echo "bes-identity: $*"; }
warn() { echo "<4>bes-identity: WARNING: $*"; }
error() { echo "<3>bes-identity: ERROR: $*"; }

# Every new identifier is a version 4 UUID from the kernel's CSPRNG, rather
# than whatever each tool would pick: early in boot some of them fall back to
# time-based UUIDs, which are neither random nor guaranteed distinct between
# machines booted at the same moment.
random_uuid() {
    cat /proc/sys/kernel/random/uuid
}

# Blocks until the kernel's random number generator is initialised, so the
# identifiers drawn from it are unpredictable.
wait_for_entropy() {
    IFS= read -r -N 1 _ </dev/random || true
}

# Waits up to $2 seconds for block device $1 to exist.
wait_for_block() {
    local path="$1" tries=$(($2 * 10))
    while [ ! -b "$path" ] && [ "$tries" -gt 0 ]; do
        sleep 0.1
        tries=$((tries - 1))
    done
    [ -b "$path" ]
}

# Prints partition number $2 of disk $1 (kernel name, e.g. vda).
partition_of() {
    local disk="$1" n="$2" p
    for p in /sys/block/"$disk"/"$disk"*; do
        if [ "$(cat "$p/partition" 2>/dev/null)" = "$n" ]; then
            echo "/dev/${p##*/}"
            return 0
        fi
    done
    return 1
}

# Sets DISK, PART1, PART2, PART3 from the root partition's label, which is
# what the rest of the system references and so what rotation keeps fixed.
resolve_layout() {
    local root_name disk_sys disk_name
    if ! wait_for_block "$ROOT_LINK" 30; then
        warn "$ROOT_LINK did not appear; nothing rotated"
        return 1
    fi
    PART3="$(readlink -f "$ROOT_LINK")"
    root_name="${PART3##*/}"
    disk_sys="$(readlink -f "/sys/class/block/$root_name/..")"
    disk_name="${disk_sys##*/}"
    DISK="/dev/$disk_name"
    if ! PART1="$(partition_of "$disk_name" 1)" || ! PART2="$(partition_of "$disk_name" 2)"; then
        warn "could not find partitions 1 and 2 on $DISK; nothing rotated"
        return 1
    fi
}

# Loads the current identifiers into CUR_<KEY>, in the record's format.
read_current() {
    local line
    while IFS= read -r line; do
        printf -v "CUR_${line%%=*}" '%s' "${line#*=}"
    done < <(identity_read "$DISK" "$PART1" "$PART2" "$PART3" "$BTRFS_DEV")
}

# The record's keys whose recorded value is no longer the current one.
rotated_keys() {
    local key rec cur
    for key in DISK_GUID PARTUUID_1 PARTUUID_2 PARTUUID_3 BOOT1_SERIAL \
        XBOOT_UUID BTRFS_UUID LUKS_UUID; do
        rec="${!key:-}"
        cur="CUR_$key"
        cur="${!cur:-}"
        if [ -n "$rec" ] && [ -n "$cur" ] && [ "${rec,,}" != "${cur,,}" ]; then
            echo "$key"
        fi
    done
}

# ------------------------------------------------------------------
# pre: partition table and LUKS
# ------------------------------------------------------------------

# r[image.identity.rotate]
rotate_gpt() {
    local cur_disk cur1 cur2 cur3
    cur_disk="$(blkid_value PTUUID "$DISK")"
    cur1="$(blkid_value PARTUUID "$PART1")"
    cur2="$(blkid_value PARTUUID "$PART2")"
    cur3="$(blkid_value PARTUUID "$PART3")"
    if [ "${cur_disk,,}" != "$DISK_GUID" ] && [ "${cur1,,}" != "$PARTUUID_1" ] &&
        [ "${cur2,,}" != "$PARTUUID_2" ] && [ "${cur3,,}" != "$PARTUUID_3" ]; then
        return 0
    fi

    log "randomising the GPT disk GUID and partition GUIDs on $DISK"
    if ! sgdisk --disk-guid="$(random_uuid)" \
        --partition-guid="1:$(random_uuid)" \
        --partition-guid="2:$(random_uuid)" \
        --partition-guid="3:$(random_uuid)" "$DISK"; then
        warn "sgdisk failed; partition table GUIDs not rotated"
        return 0
    fi

    # sgdisk asks the kernel to re-read the table. Have udev re-probe every
    # block device too, so the by-partuuid links and the partition-label
    # device units reflect the new table before anything opens a partition.
    partx -u "$DISK" 2>/dev/null
    udevadm trigger --action=change --subsystem-match=block
    udevadm settle --timeout=60

    cur3="$(blkid_value PARTUUID "$PART3")"
    if ! wait_for_block "/dev/disk/by-partuuid/${cur3,,}" 30; then
        warn "udev has not picked up the new partition GUIDs"
    fi
    if ! resolve_layout; then
        warn "the root partition did not reappear after the table was rewritten"
        return 0
    fi
    log "partition table GUIDs rotated"
}

luks_reencrypt_in_progress() {
    cryptsetup luksDump "$PART3" 2>/dev/null | grep -q '^Requirements:.*online-reencrypt'
}

# r[image.identity.luks-rekey]
reencrypt_luks() {
    local args=()
    if luks_reencrypt_in_progress; then
        log "resuming the interrupted re-encryption of $PART3"
        args+=(--resume-only)
    elif [ "$(luks_digest0 "$PART3")" = "$LUKS_DIGEST" ]; then
        log "re-encrypting $PART3 under a fresh master key; this rewrites the whole volume"
    else
        return 0
    fi
    log "interrupting this is safe: the next boot resumes where it stopped"

    if cryptsetup reencrypt "${args[@]}" --key-file "$KEYFILE" \
        --progress-frequency 10 "$PART3" </dev/null; then
        log "re-encryption complete"
        return 0
    fi

    if luks_reencrypt_in_progress; then
        error "re-encryption of $PART3 stopped part-way; not unlocking the volume."
        error "Reboot to resume it."
        return 1
    fi
    warn "re-encryption did not start; the volume is unchanged and keeps its build-time master key"
    return 0
}

# r[image.identity.rotate]
rotate_luks_uuid() {
    local cur
    cur="$(cryptsetup luksUUID "$PART3" 2>/dev/null)"
    [ "${cur,,}" = "$LUKS_UUID" ] || return 0
    log "rotating the LUKS UUID of $PART3"
    if ! cryptsetup luksUUID --batch-mode --uuid "$(random_uuid)" "$PART3"; then
        warn "LUKS UUID not rotated"
    fi
}

do_pre() {
    resolve_layout || return 0
    rotate_gpt

    [ -n "${LUKS_UUID:-}" ] || return 0
    if ! cryptsetup isLuks "$PART3"; then
        warn "the record lists a LUKS volume but $PART3 is not one; LUKS left alone"
        return 0
    fi
    mkdir -p "$STATE_DIR"
    : >"$KEYFILE"
    reencrypt_luks || return 1
    rotate_luks_uuid
}

# ------------------------------------------------------------------
# post: filesystems, grub.cfg, reboot
# ------------------------------------------------------------------

# Sets XBOOT_HAS_GRUB to yes or no from whether xboot carries a grub.cfg,
# or to unknown when xboot cannot be inspected.
inspect_xboot() {
    XBOOT_HAS_GRUB=unknown
    mkdir -p "$XBOOT_MNT"
    if ! mount -t ext4 -o ro "$PART2" "$XBOOT_MNT"; then
        warn "could not mount $PART2 to look for GRUB"
        return
    fi
    if [ -f "$XBOOT_MNT/grub/grub.cfg" ]; then
        XBOOT_HAS_GRUB=yes
    else
        XBOOT_HAS_GRUB=no
    fi
    umount "$XBOOT_MNT"
}

btrfs_fsid_change_in_progress() {
    btrfs inspect-internal dump-super "$BTRFS_DEV" 2>/dev/null | grep -q 'CHANGING_FSID'
}

# r[image.identity.rotate]
rotate_btrfs() {
    local cur
    cur="$(blkid_value UUID "$BTRFS_DEV")"
    if [ "${cur,,}" != "$BTRFS_UUID" ] && ! btrfs_fsid_change_in_progress; then
        return 0
    fi
    if [ "$XBOOT_HAS_GRUB" = unknown ]; then
        # With GRUB, a rotated btrfs ID needs grub.cfg rewritten to boot again;
        # without a readable xboot there is no telling whether that applies.
        warn "btrfs filesystem ID not rotated, since xboot could not be inspected"
        return 0
    fi
    log "rotating the btrfs filesystem ID on $BTRFS_DEV"
    # An interrupted change is resumed with the ID it was started with, which
    # btrfstune takes from the filesystem when not given one.
    local new=(-U "$(random_uuid)")
    btrfs_fsid_change_in_progress && new=(-u)
    if ! btrfstune -f "${new[@]}" "$BTRFS_DEV"; then
        warn "btrfstune failed; btrfs filesystem ID not rotated"
        return 0
    fi
    ROTATED_THIS_BOOT=yes
    # Drop the kernel's registration of the device under its old ID.
    btrfs device scan --forget >/dev/null 2>&1
    btrfs device scan "$BTRFS_DEV" >/dev/null 2>&1
}

# r[image.identity.rotate]
rotate_fat_serial() {
    local cur label serial
    cur="$(blkid_value UUID "$PART1")"
    [ "$cur" = "$BOOT1_SERIAL" ] || return 0
    log "rotating the FAT volume serial of $PART1"
    label="$(blkid_value LABEL "$PART1")"
    serial="$(random_uuid)"
    serial="${serial:0:8}"
    # mlabel rewrites the volume label along with the serial; pass the
    # current one back so it is kept.
    if MTOOLS_SKIP_CHECK=1 mlabel -N "$serial" -i "$PART1" "::$label" </dev/null; then
        ROTATED_THIS_BOOT=yes
    else
        warn "mlabel failed; FAT volume serial not rotated"
    fi
}

# r[image.identity.rotate]
rotate_xboot_uuid() {
    local cur rc
    cur="$(blkid_value UUID "$PART2")"
    [ "${cur,,}" = "$XBOOT_UUID" ] || return 0
    log "rotating the ext4 UUID of $PART2"
    e2fsck -f -y "$PART2"
    rc=$?
    if [ "$rc" -ge 4 ]; then
        warn "e2fsck found errors it could not fix (exit $rc); xboot UUID not rotated"
        return 0
    fi
    if tune2fs -U "$(random_uuid)" "$PART2"; then
        ROTATED_THIS_BOOT=yes
    else
        warn "tune2fs failed; xboot UUID not rotated"
    fi
}

# r[image.identity.grub-repair]
# Rewrites every rotated identifier in grub.cfg to its current value. The new
# file is made durable before it replaces the old one, so a power cut leaves
# either the old or the new file, never a mix.
repair_grub_cfg() {
    local cfg="$XBOOT_MNT/grub/grub.cfg" tmp key rec cur
    local exprs=()

    mkdir -p "$XBOOT_MNT"
    if ! mount -t ext4 "$PART2" "$XBOOT_MNT"; then
        error "could not mount $PART2 to repair grub.cfg"
        return 1
    fi
    if [ ! -f "$cfg" ]; then
        umount "$XBOOT_MNT"
        return 0
    fi

    for key in $(rotated_keys); do
        rec="${!key}"
        cur="CUR_$key"
        cur="${!cur}"
        case "$rec$cur" in
            *[!0-9A-Fa-f-]*)
                warn "$key has an unexpected format; not substituted in grub.cfg"
                continue
                ;;
        esac
        if grep -qiF -- "$rec" "$cfg"; then
            log "grub.cfg: replacing $key $rec with $cur"
            exprs+=(-e "s/$rec/$cur/gI")
        fi
    done

    if [ "${#exprs[@]}" -eq 0 ]; then
        umount "$XBOOT_MNT"
        return 0
    fi

    tmp="$cfg.bes-identity"
    if sed "${exprs[@]}" "$cfg" >"$tmp" &&
        chmod --reference="$cfg" "$tmp" &&
        sync "$tmp" &&
        mv -f "$tmp" "$cfg" &&
        sync "$XBOOT_MNT/grub"; then
        umount "$XBOOT_MNT"
        log "grub.cfg repaired"
        return 0
    fi
    rm -f "$tmp"
    umount "$XBOOT_MNT"
    error "could not rewrite grub.cfg"
    return 1
}

# r[image.identity.grub-repair]
# The kernel command line of this boot names identifiers as they were when
# the boot loader read its configuration; one that has since been rotated no
# longer resolves, so this boot cannot mount its root.
cmdline_references_rotated() {
    local cmdline key rec
    cmdline="$(cat /proc/cmdline)"
    for key in $(rotated_keys); do
        rec="${!key}"
        case "${cmdline,,}" in
            *"${rec,,}"*) return 0 ;;
        esac
    done
    return 1
}

do_post() {
    local stale_boot=no
    ROTATED_THIS_BOOT=no
    resolve_layout || return 0

    if cryptsetup isLuks "$PART3"; then
        BTRFS_DEV="$MAPPER_DEV"
        udevadm settle --timeout=60
        if ! wait_for_block "$BTRFS_DEV" 30; then
            warn "$BTRFS_DEV is not present; filesystem identifiers not rotated"
            return 0
        fi
    else
        BTRFS_DEV="$PART3"
    fi

    # Only look inside xboot when a step that depends on the boot loader has
    # something to do, so an already-rotated system is not touched.
    read_current
    XBOOT_HAS_GRUB=unknown
    if [ "$CUR_BTRFS_UUID" = "$BTRFS_UUID" ] || [ "$CUR_XBOOT_UUID" = "$XBOOT_UUID" ] ||
        btrfs_fsid_change_in_progress; then
        inspect_xboot
    fi

    rotate_btrfs
    rotate_fat_serial
    if [ "$XBOOT_HAS_GRUB" = no ]; then
        rotate_xboot_uuid
    fi

    read_current
    [ -n "$(rotated_keys)" ] || return 0
    cmdline_references_rotated && stale_boot=yes
    # grub.cfg can only hold a stale identifier if one was rotated this boot
    # or an earlier boot was cut off between rotating and repairing, in which
    # case the boot loader handed this boot a stale command line.
    if [ "$stale_boot" = no ] && { [ "$XBOOT_HAS_GRUB" != yes ] || [ "$ROTATED_THIS_BOOT" = no ]; }; then
        return 0
    fi

    if ! repair_grub_cfg; then
        [ "$stale_boot" = yes ] && error "this boot cannot find its root filesystem"
        return 0
    fi
    if [ "$stale_boot" = yes ]; then
        log "this boot's command line names a rotated identifier; rebooting into the repaired configuration"
        sync
        systemctl --no-block reboot
    fi
}

if [ ! -f "$RECORD" ]; then
    exit 0
fi
# shellcheck disable=SC1090 # generated record, shell-sourceable KEY=value lines
. "$RECORD"

case "${1:-}" in
    pre)
        wait_for_entropy
        do_pre
        ;;
    post)
        wait_for_entropy
        do_post
        ;;
    *)
        echo "usage: $0 pre|post" >&2
        exit 2
        ;;
esac
