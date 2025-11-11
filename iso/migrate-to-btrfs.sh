#!/bin/bash
set -e

# Migration script to move installed system from staging partition to BTRFS root
# This runs in the autoinstall late-commands phase

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

# Generate LUKS passphrase (6 words from /usr/share/dict/words)
echo "Generating LUKS encryption passphrase..."
if [ -f /usr/share/dict/words ]; then
  LUKS_PASSPHRASE=$(grep -E '^[a-z]{4,8}$' /usr/share/dict/words | shuf -n 6 | tr '\n' '-' | sed 's/-$//')
else
  # Fallback if wordlist not available
  LUKS_PASSPHRASE=$(openssl rand -base64 32)
fi
echo "$LUKS_PASSPHRASE" > /tmp/luks-passphrase

# Display passphrase to user
echo ""
echo "=========================================="
echo "DISK ENCRYPTION PASSPHRASE"
echo "=========================================="
echo ""
echo "Your disk encryption passphrase:"
echo ""
echo "$LUKS_PASSPHRASE"
echo ""
echo "This will be saved to ~/luks-passphrase.txt"
echo "IMPORTANT: Keep this safe! You may need it if disk unlock fails."
echo ""

# Setup LUKS encryption on root partition
echo "Setting up LUKS encryption..."
echo -n "$LUKS_PASSPHRASE" | cryptsetup luksFormat --type luks2 $ROOT_PART -
echo -n "$LUKS_PASSPHRASE" | cryptsetup open $ROOT_PART root-crypt -

# Create keyfile on boot partition for automatic unlock
echo "Creating keyfile for automatic unlock..."
KEYFILE_PATH="/mnt/boot-keyfile"
mkdir -p $KEYFILE_PATH
mount $BOOT_PART $KEYFILE_PATH
dd if=/dev/urandom of=$KEYFILE_PATH/luks-keyfile.key bs=1 count=64
chmod 000 $KEYFILE_PATH/luks-keyfile.key

# Add keyfile to LUKS
echo -n "$LUKS_PASSPHRASE" | cryptsetup luksAddKey $ROOT_PART $KEYFILE_PATH/luks-keyfile.key -

umount $KEYFILE_PATH
rmdir $KEYFILE_PATH

# Use the opened LUKS device
LUKS_DEV="/dev/mapper/root-crypt"

# Format LUKS container as BTRFS with features enabled
echo "Creating BTRFS filesystem with features..."
mkfs.btrfs --label ROOT --checksum xxhash --features block-group-tree,squota $LUKS_DEV

# Mount and create subvolumes
mkdir -p /mnt/newroot
mount $LUKS_DEV /mnt/newroot
btrfs quota enable --simple /mnt/newroot

echo "Creating BTRFS subvolumes..."
btrfs subvolume create /mnt/newroot/@
btrfs subvolume create /mnt/newroot/@home
btrfs subvolume create /mnt/newroot/@logs
btrfs subvolume create /mnt/newroot/@postgres
btrfs subvolume create /mnt/newroot/@containers
btrfs subvolume create /mnt/newroot/snapshots

# Copy system from staging to BTRFS subvolumes
echo "Copying system to BTRFS subvolumes..."
rsync -aAX \
  --exclude=/mnt \
  --exclude=/tmp/* \
  --exclude=/proc/* \
  --exclude=/sys/* \
  --exclude=/dev/* \
  --exclude=/home \
  --exclude=/var/log \
  / /mnt/newroot/@/

# Copy home
if [ -d /home ] && [ "$(ls -A /home 2>/dev/null)" ]; then
  echo "Copying /home..."
  rsync -aAX /home/ /mnt/newroot/@home/
fi

# Copy logs
if [ -d /var/log ] && [ "$(ls -A /var/log 2>/dev/null)" ]; then
  echo "Copying /var/log..."
  rsync -aAX /var/log/ /mnt/newroot/@logs/
fi

# Create directories for other subvolumes
mkdir -p /mnt/newroot/@/var/lib/postgresql
mkdir -p /mnt/newroot/@/var/lib/containers

# Set @ as default subvolume
echo "Setting default subvolume..."
DEFAULT_ID=$(btrfs subvolume list /mnt/newroot | grep '@$' | awk '{print $2}')
btrfs subvolume set-default $DEFAULT_ID /mnt/newroot

# Get UUIDs
LUKS_UUID=$(blkid -s UUID -o value $ROOT_PART)
ROOT_UUID=$(blkid -s UUID -o value $LUKS_DEV)
BOOT_UUID=$(blkid -s UUID -o value $BOOT_PART)
EFI_UUID=$(blkid -s UUID -o value $EFI_PART)
STAGING_UUID=$(blkid -s UUID -o value $STAGING_PART)

echo "UUIDs:"
echo "  LUKS: $LUKS_UUID"
echo "  Root: $ROOT_UUID"
echo "  Boot: $BOOT_UUID"
echo "  EFI: $EFI_UUID"
echo "  Staging: $STAGING_UUID"

# Create fstab in new root
echo "Creating /etc/fstab..."
cat > /mnt/newroot/@/etc/fstab << EOF
# /etc/fstab: static file system information
UUID=$ROOT_UUID /                       btrfs subvol=@,compress=zstd:6 0 1
UUID=$ROOT_UUID /home                   btrfs subvol=@home,compress=zstd:6 0 2
UUID=$ROOT_UUID /var/log                btrfs subvol=@logs,compress=zstd:6 0 2
UUID=$ROOT_UUID /var/lib/postgresql     btrfs subvol=@postgres,compress=zstd:6 0 2
UUID=$ROOT_UUID /var/lib/containers     btrfs subvol=@containers,compress=zstd:6 0 2
UUID=$BOOT_UUID /boot                   ext4 defaults 0 2
UUID=$EFI_UUID /boot/efi                vfat umask=0077 0 1
/dev/mapper/swap none                   swap sw 0 0
EOF

# Setup encrypted swap configuration
echo "Configuring encrypted swap..."
cat > /mnt/newroot/@/etc/crypttab << EOF
# /etc/crypttab: mappings for encrypted partitions
root-crypt UUID=$LUKS_UUID /boot/luks-keyfile.key luks,discard,keyscript=/bin/cat
swap UUID=$STAGING_UUID /dev/urandom swap,cipher=aes-xts-plain64,size=256
EOF

# Update grub to boot from new root
echo "Updating bootloader..."
mount --bind /dev /mnt/newroot/@/dev
mount --bind /proc /mnt/newroot/@/proc
mount --bind /sys /mnt/newroot/@/sys
mount $BOOT_PART /mnt/newroot/@/boot
mount $EFI_PART /mnt/newroot/@/boot/efi

# Update initramfs and grub
echo "Running update-initramfs..."
chroot /mnt/newroot/@ update-initramfs -u -k all

echo "Running update-grub..."
chroot /mnt/newroot/@ update-grub

echo "Installing GRUB..."
chroot /mnt/newroot/@ grub-install --efi-directory=/boot/efi --bootloader-id=ubuntu --recheck

# Unmount everything
echo "Unmounting filesystems..."
umount /mnt/newroot/@/boot/efi || true
umount /mnt/newroot/@/boot || true
umount /mnt/newroot/@/sys || true
umount /mnt/newroot/@/proc || true
umount /mnt/newroot/@/dev || true
umount /mnt/newroot || true
rmdir /mnt/newroot || true

# Save passphrase to user's home directory
DEFAULT_USER=$(ls /mnt/newroot/@home 2>/dev/null | head -1)
if [ -n "$DEFAULT_USER" ]; then
  echo "Saving passphrase to /home/$DEFAULT_USER/luks-passphrase.txt..."
  cp /tmp/luks-passphrase /mnt/newroot/@home/$DEFAULT_USER/luks-passphrase.txt
  chroot /mnt/newroot/@ chown $DEFAULT_USER:$DEFAULT_USER /home/$DEFAULT_USER/luks-passphrase.txt
  chroot /mnt/newroot/@ chmod 600 /home/$DEFAULT_USER/luks-passphrase.txt
fi

# Setup TPM enrollment script
cat > /mnt/newroot/@/usr/local/bin/setup-tpm-unlock << EOFTPM
#!/bin/bash
set -e

LUKS_UUID="$LUKS_UUID"
KEYFILE="/boot/luks-keyfile.key"

if [ -e /dev/tpmrm0 ] || [ -e /dev/tpm0 ]; then
  echo "TPM2 device found, enrolling for automatic unlock..."
  if [ -f /tmp/luks-passphrase ]; then
    systemd-cryptenroll --tpm2-device=auto --tpm2-pcrs=7 /dev/disk/by-uuid/\$LUKS_UUID < /tmp/luks-passphrase
    echo "TPM2 enrolled successfully with PCR 7!"

    # Remove keyfile from LUKS and securely delete it
    echo "Removing keyfile from LUKS..."
    cryptsetup luksRemoveKey /dev/disk/by-uuid/\$LUKS_UUID \$KEYFILE

    # Secure delete keyfile
    echo "Securely deleting keyfile..."
    shred -vfz -n 3 \$KEYFILE
    rm -f \$KEYFILE

    # Update crypttab to remove keyfile reference
    sed -i "s|root-crypt UUID=\$LUKS_UUID /boot/luks-keyfile.key luks,discard,keyscript=/bin/cat|root-crypt UUID=\$LUKS_UUID none luks,discard|" /etc/crypttab

    echo "Keyfile removed. System will auto-unlock via TPM on next boot."
    rm -f /tmp/luks-passphrase
  else
    echo "WARNING: Could not find passphrase file, skipping TPM enrollment"
  fi
else
  echo "No TPM2 device found. System will use keyfile at \$KEYFILE for auto-unlock."
  echo "Passphrase fallback available if keyfile is unavailable."
  rm -f /tmp/luks-passphrase
fi
EOFTPM

chmod +x /mnt/newroot/@/usr/local/bin/setup-tpm-unlock

# Create systemd service for TPM enrollment
cat > /mnt/newroot/@/etc/systemd/system/setup-tpm-unlock.service << 'EOFTPMSVC'
[Unit]
Description=Setup TPM2 auto-unlock for LUKS
After=local-fs.target
Before=tailscale-first-boot.service
ConditionPathExists=!/etc/tpm-enrolled

[Service]
Type=oneshot
ExecStart=/usr/local/bin/setup-tpm-unlock
ExecStartPost=/usr/bin/touch /etc/tpm-enrolled
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOFTPMSVC

# Enable TPM enrollment service
chroot /mnt/newroot/@ systemctl enable setup-tpm-unlock.service

# Copy passphrase for TPM enrollment on first boot
cp /tmp/luks-passphrase /mnt/newroot/@/tmp/luks-passphrase 2>/dev/null || true
