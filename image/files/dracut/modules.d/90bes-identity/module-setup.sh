#!/bin/bash
# dracut module: first-boot rotation of the image's build-time identifiers.
# Called by dracut, which provides $moddir, $initdir, $systemdsystemunitdir,
# $SYSTEMCTL, $dracutsysrootdir and the inst_* / instmods helpers.
# shellcheck disable=SC2154 # the variables above are set by dracut

RECORD=/etc/bes/build-identity

check() {
    # Without a record there is nothing to compare against.
    [[ -f "${dracutsysrootdir-}$RECORD" ]] || return 1
    require_binaries sgdisk partx blkid btrfs btrfstune e2fsck tune2fs mlabel cryptsetup || return 1
    return 0
}

depends() {
    echo systemd btrfs
    if grep -q '^LUKS_UUID=' "${dracutsysrootdir-}$RECORD"; then
        echo crypt
    fi
}

installkernel() {
    # Offline re-encryption runs its cipher through the kernel crypto API
    # from userspace.
    if grep -q '^LUKS_UUID=' "${dracutsysrootdir-}$RECORD"; then
        hostonly='' instmods af_alg algif_skcipher dm_crypt
    fi
}

install() {
    # r[image.identity.record]
    inst_simple "$RECORD"

    inst_simple "$moddir/bes-identity-lib.sh" /usr/lib/bes-identity/bes-identity-lib.sh
    inst_script "$moddir/bes-identity.sh" /usr/bin/bes-identity
    inst_multiple sgdisk partx blkid udevadm cryptsetup btrfs btrfstune e2fsck tune2fs \
        mlabel awk grep sed cat readlink sleep mkdir mount umount sync mv rm chmod systemctl

    # mlabel converts the volume label through its default codepage, CP850,
    # which glibc loads as a gconv module; without it mlabel cannot open the
    # volume at all.
    local gconv
    for gconv in /usr/lib/*/gconv /usr/lib64/gconv /usr/lib/gconv; do
        [[ -f "${dracutsysrootdir-}$gconv/IBM850.so" ]] || continue
        inst_multiple "$gconv/IBM850.so" "$gconv/gconv-modules"
        inst_multiple -o "$gconv/gconv-modules.d/*.conf"
        break
    done

    inst_simple "$moddir/bes-identity-pre.service" "$systemdsystemunitdir/bes-identity-pre.service"
    inst_simple "$moddir/bes-identity-post.service" "$systemdsystemunitdir/bes-identity-post.service"
    inst_simple "$moddir/systemd-cryptsetup-root.conf" \
        "$systemdsystemunitdir/systemd-cryptsetup@root.service.d/50-bes-identity.conf"
    $SYSTEMCTL -q --root "$initdir" enable bes-identity-pre.service bes-identity-post.service
}
