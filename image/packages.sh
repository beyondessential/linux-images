#!/bin/bash
PACKAGES=(
    # Kernel and boot
    linux-generic
    grub-efi
    dracut-core

    # Filesystem and storage
    btrfs-progs
    cryptsetup
    snapper
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

    # for dracut modules
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
