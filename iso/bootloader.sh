#!/bin/bash
set -ex

apt install -y dracut # removes initramfs-tools
dracut -H --hostonly-mode=sloppy --force --kver $(ls /lib/modules | head -n1)

sed -i 's/GRUB_TIMEOUT=0/GRUB_TIMEOUT=5/' /etc/default/grub
sed -i 's/GRUB_TIMEOUT_STYLE=hidden/GRUB_TIMEOUT_STYLE=menu/' /etc/default/grub
sed -i 's/GRUB_CMDLINE_LINUX_DEFAULT=""/GRUB_CMDLINE_LINUX_DEFAULT="noresume"/' /etc/default/grub

ROOT_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'root' | awk '{print $1}')"
echo GRUB_FORCE_PARTUUID="$(blkid -s PARTUUID -o value $ROOT_PART)" >> /etc/default/grub

rm -rf /boot/grub || true
mkdir /boot/grub
update-grub
grub-install --bootloader-id=ubuntu
