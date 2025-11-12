#!/bin/bash
set -ex

sed -i 's/GRUB_TIMEOUT=.+/GRUB_TIMEOUT=5/' /etc/default/grub
sed -i 's/GRUB_TIMEOUT_STYLE=.+/GRUB_TIMEOUT_STYLE=menu/' /etc/default/grub
echo GRUB_DISABLE_LINUX_UUID=true >> /etc/default/grub
echo GRUB_DISABLE_LINUX_PARTUUID=false >> /etc/default/grub

rm -rf /boot/grub || true
update-initramfs -u -k all
update-grub
grub-install --bootloader-id=ubuntu
