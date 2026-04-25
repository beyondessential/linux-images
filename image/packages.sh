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

    # Time synchronization (replaces systemd-timesyncd, which is shipped as a
    # noble dependency of systemd-sysv but no longer exists on 26.04+)
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
