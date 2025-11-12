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

: Generating LUKS encryption passphrase
if [ -f /usr/share/dict/words ]; then
  LUKS_PASSPHRASE=$(grep -E '^[a-z]{4,8}$' /usr/share/dict/words | shuf -n 6 | tr '\n' '-' | sed 's/-$//')
else
  # Fallback if wordlist not available
  LUKS_PASSPHRASE=$(openssl rand -base64 32)
fi
echo "$LUKS_PASSPHRASE" > /tmp/luks-passphrase

: Setup LUKS volume on real root
KEYFILE=/boot/.luks.key
dd if=/dev/random of=$KEYFILE bs=1 count=64
chmod 000 $KEYFILE
cryptsetup luksFormat --type luks2 $ROOT_PART --key-file=$KEYFILE

: Setup passphrase
echo -n "$LUKS_PASSPHRASE" | cryptsetup luksAddKey $ROOT_PART --key-file=$KEYFILE -

: Open LUKS device
cryptsetup open $ROOT_PART root --key-file=$KEYFILE
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
install -m600 /tmp/luks-passphrase /mnt/newroot/@/root/luks-passphrase.txt

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
root PARTUUID=$LUKS_UUID /.luks.key:PARTUUID=$BOOT_UUID luks,discard
swap PARTUUID=$STAGING_UUID /dev/urandom swap,cipher=aes-xts-plain64
EOF

: Write fstab
cat > /mnt/newroot/@/etc/fstab << EOF
/dev/mapper/root /                       btrfs subvol=@,compress=zstd:6 0 1
/dev/mapper/root /home                   btrfs subvol=@home,compress=zstd:6 0 2
/dev/mapper/root /var/log                btrfs subvol=@logs,compress=zstd:6 0 2
/dev/mapper/root /var/lib/postgresql     btrfs subvol=@postgres,compress=zstd:6 0 2
/dev/mapper/root /var/lib/containers     btrfs subvol=@containers,compress=zstd:6 0 2
/dev/mapper/root /.snapshots             btrfs subvol=@.snapshots,compress=zstd:6 0 2
PARTUUID=$BOOT_UUID /boot                ext4 defaults 0 2
PARTUUID=$EFI_UUID /boot/efi             vfat umask=0077 0 1
/dev/mapper/swap none                    swap sw 0 0
EOF

: Setup TPM enrollment script
cat > /mnt/newroot/@/usr/local/bin/setup-tpm-unlock << EOFTPM
#!/bin/bash
set -e

KEYFILE="/boot/.luks.key"

if [ -e /dev/tpmrm0 ] || [ -e /dev/tpm0 ]; then
  echo "TPM2 device found, enrolling for automatic unlock..."
  systemd-cryptenroll --tpm2-device=auto --tpm2-pcrs=7 /dev/disk/by-partuuid/$LUKS_UUID --unlock-key-file=\$KEYFILE
  echo "TPM2 enrolled successfully!"

  echo "Removing keyfile from LUKS..."
  cryptsetup luksRemoveKey /dev/disk/by-partuuid/$LUKS_UUID \$KEYFILE

  echo "Securely deleting keyfile..."
  shred -vfz -n 3 \$KEYFILE
  rm -f \$KEYFILE

  # Update crypttab to remove keyfile reference
  sed -i "s|root .+|root PARTUUID=$LUKS_UUID none luks,discard|" /etc/crypttab
else
  echo "No TPM2 device found. System will use keyfile at \$KEYFILE for auto-unlock."
  echo "Passphrase fallback available if keyfile is unavailable."
fi
EOFTPM

chmod +x /mnt/newroot/@/usr/local/bin/setup-tpm-unlock

: Create systemd service for TPM enrollment
cat > /mnt/newroot/@/etc/systemd/system/setup-tpm-unlock.service << 'EOFTPMSVC'
[Unit]
Description=Setup TPM2 auto-unlock for LUKS
After=local-fs.target
Before=tailscale-first-boot.service
ConditionPathExists=/boot/.luks.key

[Service]
Type=oneshot
ExecStart=/usr/local/bin/setup-tpm-unlock
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOFTPMSVC

: Enable TPM enrollment service
chroot /mnt/newroot/@ systemctl enable setup-tpm-unlock.service
