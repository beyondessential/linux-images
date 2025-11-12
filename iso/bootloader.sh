#!/bin/bash
set -ex

sed -i 's/GRUB_TIMEOUT=0/GRUB_TIMEOUT=5/' /etc/default/grub
sed -i 's/GRUB_TIMEOUT_STYLE=hidden/GRUB_TIMEOUT_STYLE=menu/' /etc/default/grub
sed -i 's/GRUB_CMDLINE_LINUX_DEFAULT=""/GRUB_CMDLINE_LINUX_DEFAULT="noresume"/' /etc/default/grub
echo GRUB_DISABLE_LINUX_UUID=true >> /etc/default/grub
echo GRUB_DISABLE_LINUX_PARTUUID=false >> /etc/default/grub

rm -rf /boot/grub || true
mkdir /boot/grub
update-initramfs -u -k all
update-grub
grub-install --bootloader-id=ubuntu
