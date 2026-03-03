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

    # for growpart
    cloud-guest-utils
    parted

    # Networking
    openssh-server
    curl
    wget
    ufw

    # Cloud
    cloud-init

    # System
    systemd-resolved
    rsync
    cron
    sudo

    # for dracut modules
    tpm2-tools
    nvme-cli
    busybox
    rng-tools5
    jq

    # r[image.packages.caddy]: from bes-tools repo
    caddy

    # r[image.packages.podman]: from bes-tools repo
    podman

    # r[image.packages.kopia]: from official Kopia repo
    kopia

    # r[image.packages.bestool]: from bes-tools repo
    bestool

    # Editors and tools (it's really annoying not having these)
    neovim
    nano
    less
    htop
    iputils-ping
)
