#!/bin/bash
set -ex

# this will be the default in the next LTS, but we want to use
# it today because otherwise the disk encryption doesn't work.
apt install -y dracut # removes initramfs-tools

# this is fixed (included by default) in 25.10+
cat > /etc/dracut.conf.d/01-fix-hostonly-noble.conf <<EOF
hostonly="yes"
hostonly_mode="sloppy"
EOF

# without this, dracut doesn't include the luks empty password option
cat > /etc/dracut.conf.d/99-luks-empty-password.conf <<EOF
kernel_cmdline+=" rd.luks.options=discard,try-empty-password=true "
EOF

dracut --force --kver $(ls /lib/modules | head -n1)

sed -i 's/GRUB_TIMEOUT=0/GRUB_TIMEOUT=5/' /etc/default/grub
sed -i 's/GRUB_TIMEOUT_STYLE=hidden/GRUB_TIMEOUT_STYLE=menu/' /etc/default/grub
sed -i 's/GRUB_CMDLINE_LINUX_DEFAULT=""/GRUB_CMDLINE_LINUX_DEFAULT="noresume"/' /etc/default/grub
echo GRUB_RECORDFAIL_TIMEOUT=5 >> /etc/default/grub

rm -rf /boot/grub || true
mkdir /boot/grub
update-grub
grub-install --bootloader-id=ubuntu
