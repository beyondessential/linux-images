# shellcheck shell=bash
# Shared build-identity readers, sourced by image/build.sh,
# tests/test-image-structure.sh, and the 90bes-identity dracut module's
# bes-identity script. Needs bash, blkid, cryptsetup, and awk, all of which
# are present wherever this is sourced (the dracut module installs them into
# the initramfs alongside this file).

# Reads one blkid tag from a device, probing the device itself rather than
# trusting blkid's cache, which can lag behind a change made seconds ago.
blkid_value() {
    blkid -c /dev/null -o value -s "$1" "$2" 2>/dev/null
}

# Extracts the hex digest bytes for LUKS2 digest 0 from `cryptsetup
# luksDump` text output. Not JSON: jq isn't a guaranteed dependency.
luks_digest0() {
    cryptsetup luksDump "$1" | awk '
        /^Digests:/ { section = 1; next }
        section && /^[[:space:]]*[0-9]+:/ {
            id = $0
            sub(/^[[:space:]]*/, "", id)
            sub(/:.*/, "", id)
            cur = id
            capture = 0
            next
        }
        section && cur == "0" && /Digest:/ {
            line = $0
            sub(/.*Digest:[[:space:]]*/, "", line)
            digest = digest line
            capture = 1
            next
        }
        section && cur == "0" && capture && /^[[:space:]]+[0-9a-f]{2}([[:space:]][0-9a-f]{2})*[[:space:]]*$/ {
            digest = digest $0
            next
        }
        { if (capture) capture = 0 }
        END {
            gsub(/[[:space:]]/, "", digest)
            printf "%s", digest
        }
    '
}

# Prints the identifiers named in r[image.identity.record] as they currently
# stand on a disk, in the record's own KEY=value format. The LUKS keys are
# printed only when the root partition holds a LUKS volume.
#
# Usage: identity_read <disk> <boot1-part> <xboot-part> <root-part> <btrfs-dev>
identity_read() {
    local disk="$1" boot1="$2" xboot="$3" root="$4" btrfs_dev="$5" v

    v="$(blkid_value PTUUID "$disk")"
    echo "DISK_GUID=${v,,}"
    v="$(blkid_value PARTUUID "$boot1")"
    echo "PARTUUID_1=${v,,}"
    v="$(blkid_value PARTUUID "$xboot")"
    echo "PARTUUID_2=${v,,}"
    v="$(blkid_value PARTUUID "$root")"
    echo "PARTUUID_3=${v,,}"
    echo "BOOT1_SERIAL=$(blkid_value UUID "$boot1")"
    v="$(blkid_value UUID "$xboot")"
    echo "XBOOT_UUID=${v,,}"
    v="$(blkid_value UUID "$btrfs_dev")"
    echo "BTRFS_UUID=${v,,}"
    if cryptsetup isLuks "$root" 2>/dev/null; then
        v="$(cryptsetup luksUUID "$root")"
        echo "LUKS_UUID=${v,,}"
        v="$(luks_digest0 "$root")"
        echo "LUKS_DIGEST=${v,,}"
    fi
}
