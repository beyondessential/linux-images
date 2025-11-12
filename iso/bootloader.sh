#!/bin/bash
set -ex

update-initramfs -u -k all
update-grub
grub-install --efi-directory=/boot/efi --bootloader-id=ubuntu --recheck
