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
        PACKAGES+=(
            linux-raspi
            linux-firmware-raspi
            i2c-tools
        )
        ;;
    *)
        echo "ERROR: packages.sh: unknown VARIANT=${VARIANT:-<unset>}" >&2
        return 1
        ;;
esac

# On Ubuntu 25.10+ (resolute and later), /usr/lib/systemd/systemd-cryptsetup
# moved out of the systemd package into its own. It is only a Recommends of
# systemd (and a Suggests of dracut-core); with --no-install-recommends,
# neither path pulls it in. Without that binary dracut's 71systemd-cryptsetup
# module's check() fails, the module is dropped, and the initramfs cannot
# unlock LUKS at boot. The package doesn't exist on noble (24.04), where the
# binary still ships inside systemd, so install it conditionally.
if [ "${UBUNTU_SUITE:-noble}" != "noble" ]; then
    PACKAGES+=(systemd-cryptsetup)
fi
