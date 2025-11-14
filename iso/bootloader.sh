#!/bin/bash
set -euxo pipefail

# this will be the default in the next LTS, but we want to use
# it today because otherwise the disk encryption doesn't work.
apt install -y dracut # removes initramfs-tools

# this is fixed (included by default) in 25.10+
cat > /etc/dracut.conf.d/01-fix-hostonly-noble.conf <<EOF
hostonly="yes"
hostonly_mode="sloppy"
EOF

: Create empty keyfile for LUKS
KEYFILE_PATH="/etc/luks/empty-keyfile"
mkdir -p /etc/luks
touch "$KEYFILE_PATH"
chmod 000 "$KEYFILE_PATH"

: Add keyfile to dracut
cat > /etc/dracut.conf.d/02-luks-keyfile.conf <<EOF
install_items+=" $KEYFILE_PATH "
EOF

: Adjust grub settings
sed -i 's/GRUB_TIMEOUT=0/GRUB_TIMEOUT=5/' /etc/default/grub
sed -i 's/GRUB_TIMEOUT_STYLE=hidden/GRUB_TIMEOUT_STYLE=menu/' /etc/default/grub
sed -i 's/GRUB_CMDLINE_LINUX_DEFAULT=""/GRUB_CMDLINE_LINUX_DEFAULT="noresume"/' /etc/default/grub
echo GRUB_RECORDFAIL_TIMEOUT=5 >> /etc/default/grub

: Write crypttab
cat > /etc/crypttab << EOF
# <name> <device>                    <keyfile>                 <options>
root     /dev/disk/by-partlabel/root /etc/luks/empty-keyfile  force,luks,discard,headless=true,try-empty-password=true
swap     /dev/disk/by-partlabel/swap /dev/urandom             swap,cipher=aes-xts-plain64
EOF
# the "force" is needed for dracut to pickup the entry as it skips keyfile'd entries by default
# why? nobody knows. https://github.com/dracutdevs/dracut/issues/2128#issuecomment-1353362957

: Write fstab
cat > /etc/fstab << EOF
# <device>                   <mountpoint>        <fs>  <options>                     <dump> <pass>
/dev/mapper/root             /                   btrfs subvol=@,compress=zstd:6           0 1
/dev/mapper/root             /home               btrfs subvol=@home,compress=zstd:6       0 2
/dev/mapper/root             /var/log            btrfs subvol=@logs,compress=zstd:6       0 2
/dev/mapper/root             /var/lib/postgresql btrfs subvol=@postgres,compress=zstd:6   0 2
/dev/mapper/root             /var/lib/containers btrfs subvol=@containers,compress=zstd:6 0 2
/dev/mapper/root             /.snapshots         btrfs subvol=@.snapshots,compress=zstd:6 0 2
/dev/disk/by-partlabel/xboot /boot               ext4  defaults                           0 2
/dev/disk/by-partlabel/efi   /boot/efi           vfat  umask=0077                         0 1
/dev/mapper/swap             none                swap  sw                                 0 0
EOF

: Setup TPM enrollment script
cat > /usr/local/bin/setup-tpm-unlock << EOFTPM
#!/bin/bash
set -euxo pipefail
systemd-cryptenroll --wipe-slot=0 --tpm2-device=auto --tpm2-pcrs=7 /dev/disk/by-partlabel/root --unlock-key-file=/etc/luks/empty-keyfile
sed -i "s|/etc/luks/empty-keyfile|-|" /etc/crypttab
sed -i "s|try-empty-password=true|tpm2-device=auto|" /etc/crypttab
touch /etc/luks/tpm-enrolled
dracut -f
EOFTPM
chmod +x /usr/local/bin/setup-tpm-unlock

: Create systemd service for TPM enrollment
cat > /etc/systemd/system/setup-tpm-unlock.service << 'EOFTPMSVC'
[Unit]
Description=Setup TPM2 auto-unlock for LUKS
After=local-fs.target
ConditionPathExists=/dev/tpm0
ConditionPathExists=/dev/tpmrm0
ConditionPathExists=!/etc/luks/tpm-enrolled

[Service]
Type=oneshot
ExecStart=/usr/local/bin/setup-tpm-unlock
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOFTPMSVC

: Enable TPM enrollment service
systemctl enable setup-tpm-unlock.service

: Regenerate boot files
dracut --force --kver $(ls /lib/modules | head -n1)
rm -rf /boot/grub || true
mkdir /boot/grub
update-grub
grub-install --bootloader-id=ubuntu
