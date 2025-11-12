#!/bin/bash
set -ex

# Find partitions
DISK=$(lsblk -ndo PKNAME $(findmnt -n -o SOURCE /))
ROOT_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'root' | awk '{print $1}')"
STAGING_PART=$(findmnt -n -o SOURCE /)
BOOT_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'xboot' | awk '{print $1}' | head -1)"
EFI_PART="/dev/$(lsblk -ln -o NAME,PARTLABEL | grep 'efi' | awk '{print $1}')"

if [ "$ROOT_PART" = "/dev/" ]; then
  echo "Partitioning failed"
  exit 1
fi

echo "Disk: $DISK"
echo "Root partition: $ROOT_PART"
echo "Staging partition: $STAGING_PART"
echo "Boot partition: $BOOT_PART"
echo "EFI partition: $EFI_PART"

: Setup LUKS volume on real root
KEYFILE=/tmp/empty-passphrase
touch /tmp/empty-passphrase
cryptsetup luksFormat --type luks2 $ROOT_PART --key-file $KEYFILE --key-slot 10

: Open LUKS device
cryptsetup open $ROOT_PART root --key-file $KEYFILE
LUKS_DEV="/dev/mapper/root"

: Create filesystem
mkfs.btrfs --label ROOT --checksum xxhash --features block-group-tree,squota $LUKS_DEV

mkdir -p /mnt/newroot
mount $LUKS_DEV /mnt/newroot
btrfs quota enable --simple /mnt/newroot

: Create subvolumes
btrfs subvolume create /mnt/newroot/@
btrfs subvolume create /mnt/newroot/@home
btrfs subvolume create /mnt/newroot/@logs
btrfs subvolume create /mnt/newroot/@postgres
btrfs subvolume create /mnt/newroot/@containers
btrfs subvolume create /mnt/newroot/@.snapshots

: Copy system from staging to real root
rsync -aAX \
  --exclude=/mnt \
  --exclude=/cdrom \
  --exclude=/boot/\* \
  --exclude=/tmp/\* \
  --exclude=/proc/\* \
  --exclude=/sys/\* \
  --exclude=/dev/\* \
  --exclude=/home \
  --exclude=/var/log \
  / /mnt/newroot/@/

if [ -d /home ] && [ "$(ls -A /home 2>/dev/null)" ]; then
  : Copying /home
  rsync -aAX /home/ /mnt/newroot/@home/
fi

if [ -d /var/log ] && [ "$(ls -A /var/log 2>/dev/null)" ]; then
  : Copying /var/log
  rsync -aAX /var/log/ /mnt/newroot/@logs/
fi

: Create mountpoints
mkdir -p /mnt/newroot/@/{.snapshots,boot,dev,home,mnt,proc,root,run,sys,tmp,var/{lib/{postgresql,containers},log}}
: Pre-create directories for snapshots
mkdir -p /mnt/newroot/@snapshots/{root,home,logs,postgres,containers}
: Save passphrase to real root

LUKS_UUID=$(blkid -s PARTUUID -o value $ROOT_PART)
ROOT_UUID=$(blkid -s PARTUUID -o value $LUKS_DEV)
BOOT_UUID=$(blkid -s PARTUUID -o value $BOOT_PART)
EFI_UUID=$(blkid -s PARTUUID -o value $EFI_PART)
STAGING_UUID=$(blkid -s PARTUUID -o value $STAGING_PART)

echo "UUIDs:"
echo "  LUKS: $LUKS_UUID"
echo "  Root: $ROOT_UUID"
echo "  Boot: $BOOT_UUID"
echo "  EFI: $EFI_UUID"
echo "  Staging: $STAGING_UUID"

: Write crypttab
cat > /mnt/newroot/@/etc/crypttab << EOF
# <name> <device>       <keyfile>    <options>
root     PARTLABEL=root /dev/null    luks,discard,headless=true,try-empty-password=true
swap     PARTLABEL=swap /dev/urandom swap,cipher=aes-xts-plain64
EOF

: Write fstab
cat > /mnt/newroot/@/etc/fstab << EOF
# <device>       <mountpoint>            <fs>  <options>                     <dump> <pass>
/dev/mapper/root /                       btrfs subvol=@,compress=zstd:6           0 1
/dev/mapper/root /home                   btrfs subvol=@home,compress=zstd:6       0 2
/dev/mapper/root /var/log                btrfs subvol=@logs,compress=zstd:6       0 2
/dev/mapper/root /var/lib/postgresql     btrfs subvol=@postgres,compress=zstd:6   0 2
/dev/mapper/root /var/lib/containers     btrfs subvol=@containers,compress=zstd:6 0 2
/dev/mapper/root /.snapshots             btrfs subvol=@.snapshots,compress=zstd:6 0 2
PARTLABEL=xboot  /boot                   ext4  defaults                           0 2
PARTLABEL=efi    /boot/efi               vfat  umask=0077                         0 1
/dev/mapper/swap none                    swap  sw                                 0 0
EOF

: Setup TPM enrollment script
cat > /mnt/newroot/@/usr/local/bin/setup-tpm-unlock << EOFTPM
#!/bin/bash
set -e

if [ -e /dev/tpm* ]; then
  echo "TPM2 device found, enrolling for automatic unlock..."
  touch /tmp/empty-passphrase
  systemd-cryptenroll --wipe-slot=10 --tpm2-device=auto --tpm2-pcrs=7 /dev/disk/by-partlabel/root --unlock-key-file=/tmp/empty-passphrase
  sed -i "s|try-empty-password=true|tpm2-device=auto|" /etc/crypttab
  touch /etc/tpm-enrolled
  echo "TPM2 enrolled successfully!"
else
  echo "No TPM2 device found."
fi
EOFTPM

chmod +x /mnt/newroot/@/usr/local/bin/setup-tpm-unlock

: Create systemd service for TPM enrollment
cat > /mnt/newroot/@/etc/systemd/system/setup-tpm-unlock.service << 'EOFTPMSVC'
[Unit]
Description=Setup TPM2 auto-unlock for LUKS
After=local-fs.target
Before=tailscale-first-boot.service
ConditionPathExists=!/etc/tpm-enrolled

[Service]
Type=oneshot
ExecStart=/usr/local/bin/setup-tpm-unlock
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOFTPMSVC

: Enable TPM enrollment service
chroot /mnt/newroot/@ systemctl enable setup-tpm-unlock.service
