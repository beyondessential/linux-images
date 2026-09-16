#!/bin/bash
# Package lists are sourced into configure.sh, which decides which to apply
# based on $VARIANT. Common packages always go in; the variant-specific list
# layers on top.

PACKAGES=(
    # Filesystem and storage
    btrfs-progs
    cryptsetup
    gdisk
    mtools

    # for growpart
    cloud-guest-utils
    parted

    # Networking
    netplan.io
    openssh-server
    curl
    wget
    ufw

    # Cloud
    cloud-init

    # r[impl image.cloud-init.apt-codename]
    # cloud-init resolves the suite codename by executing lsb_release rather
    # than reading /etc/os-release. Without this package the lookup fails and
    # it writes "UNAVAILABLE" into the APT suites it generates on first boot,
    # leaving apt unable to fetch anything on every deployed instance.
    lsb-release

    # Time synchronization. systemd-timesyncd is only a Recommends of
    # systemd-sysv, so --no-install-recommends leaves the image without any
    # time-sync daemon by default. Ship chrony explicitly so first boot has
    # working NTP.
    chrony

    # System
    systemd-resolved
    rsync
    cron
    sudo

    # r[image.packages.service-restart+2]
    # Restart services after upgrades. Pulled in by the server task on a stock
    # install, but this is a --no-install-recommends minbase build, so it must
    # be listed explicitly. Configured for non-interactive auto-restart in
    # configure.sh.
    needrestart

    # APT key management
    gnupg

    # Initramfs (variant adds the bootloader/kernel; dracut is common)
    dracut-core

    # for dracut modules. tpm2-tools is in common because, while x86 server
    # hardware and Pi 5 add-ons (SLB9670 / similar) differ wildly, both
    # benefit from the userspace tooling when a TPM is present, and on
    # systems without one the package is small and inert.
    tpm2-tools
    nvme-cli
    busybox
    rng-tools5
    jq

    # Console font
    console-setup
    kbd

    # Editors and tools (it's really annoying not having these)
    neovim
    nano
    less
    htop
    iputils-ping
)

# r[image.variant.types+3]
case "${VARIANT:-}" in
    metal|cloud)
        PACKAGES+=(
            linux-generic
            grub-efi
        )
        ;;
    pi)
        # r[image.boot.pi-peripherals]
        # i2c-tools pairs with dtparam=i2c_arm=on in config.txt for sensor /
        # peripheral work over the GPIO header. tpm2-tools comes from the
        # common list above; Pi 5 has no native TPM but we ship-with-overlay
        # for an optional SPI TPM HAT (see r[image.boot.pi-tpm-overlay]).
        # flash-kernel-piboot is installed separately in configure.sh so
        # configure.sh can lay out /boot/firmware/current/ before the package
        # is dropped in — the chroot build doesn't run flash-kernel itself
        # (see configure.sh for the A/B layout, r[image.boot.pi-tryboot-rollback]).
        #
        # r[image.wireless.pi-bluetooth] r[image.wireless.pi-wifi]
        # Both radios' firmware already arrives with linux-firmware-raspi, and
        # the Pi 5 DTB carries the Bluetooth controller as a serdev child of
        # its UART, so the kernel binds it with no attach helper — the
        # pi-bluetooth package and its btuart/bthelper serve the older
        # pre-serdev path and are not needed here. What is missing is purely
        # userspace. The bluetooth metapackage names the host stack; bluez is
        # its only dependency, and the companions it lists (bluez-cups,
        # bluez-meshd, bluez-obexd) are Suggests, which apt does not act on,
        # so nothing arrives that is not asked for here. wpasupplicant has to
        # be named because netplan.io does not depend on, recommend, or even
        # suggest it, so nothing else in the graph would pull it in, and
        # without it only an open network can be joined. wireless-regdb gives
        # the kernel channel and transmit-power limits to apply once a
        # regulatory domain is set; without it there is no database to apply.
        #
        # iw and rfkill are diagnostics, in the same spirit as i2c-tools. The
        # expected workload drives Bluetooth Low Energy through bluetoothd's
        # D-Bus API from a purpose-built tool, which needs neither; they are
        # here for the times that tool misbehaves. Bluetooth wants nothing
        # extra of its own: bluez already carries btmon, which traces the HCI
        # transport and so reports what the controller was actually told,
        # whatever the tool believes it sent. On a headless board rfkill is
        # also the only way to tell a dead radio from a soft-blocked one.
        PACKAGES+=(
            linux-raspi
            linux-firmware-raspi
            i2c-tools

            bluetooth
            wpasupplicant
            wireless-regdb
            iw
            rfkill
        )
        ;;
    *)
        echo "ERROR: packages.sh: unknown VARIANT=${VARIANT:-<unset>}" >&2
        return 1
        ;;
esac

# /usr/lib/systemd/systemd-cryptsetup ships in its own package rather than in
# systemd. It is only a Recommends of systemd (and a Suggests of dracut-core);
# with --no-install-recommends, neither path pulls it in. Without that binary
# dracut's 71systemd-cryptsetup module's check() fails, the module is dropped,
# and the initramfs cannot unlock LUKS at boot.
PACKAGES+=(systemd-cryptsetup)
