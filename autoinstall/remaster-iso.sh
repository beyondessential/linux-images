#!/bin/bash
set -euo pipefail

# ISO remastering script to create Ubuntu installation media with embedded autoinstall
# Creates a custom ISO that automatically installs with our BTRFS + encrypted swap config

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UBUNTU_VERSION="24.04.3"
WORK_DIR="${WORK_DIR:-/tmp/ubuntu-remaster}"

usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Create a custom Ubuntu ISO with embedded autoinstall configuration

OPTIONS:
    -a, --arch ARCH          Architecture: amd64 or arm64 (default: amd64)
    -i, --input ISO          Input ISO file (if not provided, will download)
    -o, --output ISO         Output ISO file (default: ubuntu-24.04-autoinstall-ARCH.iso)
    -w, --work-dir DIR       Working directory (default: /tmp/ubuntu-remaster)
    -k, --keep-work          Keep working directory after completion
    -h, --help               Show this help

EXAMPLES:
    # Create AMD64 ISO (will download Ubuntu ISO)
    $0 --arch amd64

    # Use existing ISO
    $0 --input ubuntu-24.04-server-amd64.iso --output custom.iso

    # Create ARM64 ISO
    $0 --arch arm64

REQUIREMENTS:
    - xorriso or genisoimage
    - 7z or bsdtar
    - wget or curl
    - sudo access (for mounting)

EOF
    exit 0
}

# Parse arguments
ARCH="amd64"
INPUT_ISO=""
OUTPUT_ISO=""
KEEP_WORK=0

while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--arch)
            ARCH="$2"
            shift 2
            ;;
        -i|--input)
            INPUT_ISO="$2"
            shift 2
            ;;
        -o|--output)
            OUTPUT_ISO="$2"
            shift 2
            ;;
        -w|--work-dir)
            WORK_DIR="$2"
            shift 2
            ;;
        -k|--keep-work)
            KEEP_WORK=1
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown option: $1"
            usage
            ;;
    esac
done

# Set defaults
OUTPUT_ISO="${OUTPUT_ISO:-ubuntu-${UBUNTU_VERSION}-autoinstall-${ARCH}.iso}"
ISO_EXTRACT="$WORK_DIR/extract"
ISO_MOUNT="$WORK_DIR/mount"
ISO_BUILD="$WORK_DIR/build"

# Validate architecture
if [[ "$ARCH" != "amd64" && "$ARCH" != "arm64" ]]; then
    echo "ERROR: Architecture must be amd64 or arm64"
    exit 1
fi

# Check dependencies
check_deps() {
    local missing=0
    for cmd in xorriso wget; do
        if ! command -v "$cmd" &> /dev/null; then
            echo "ERROR: $cmd not found"
            missing=1
        fi
    done
    if [ $missing -eq 1 ]; then
        echo "Install with: sudo apt-get install xorriso wget"
        exit 1
    fi
}

check_deps

# Download ISO if not provided
if [ -z "$INPUT_ISO" ]; then
    echo "No input ISO specified, downloading Ubuntu ${UBUNTU_VERSION} ${ARCH}..."
    ISO_NAME="ubuntu-${UBUNTU_VERSION}-live-server-${ARCH}.iso"
    INPUT_ISO="/tmp/${ISO_NAME}"

    # Check if cached ISO exists and is non-zero
    if [ -f "$INPUT_ISO" ]; then
        if [ ! -s "$INPUT_ISO" ]; then
            echo "Found zero-sized cached ISO, removing: $INPUT_ISO"
            rm -f "$INPUT_ISO"
        else
            echo "Using cached ISO: $INPUT_ISO"
        fi
    fi

    # Download if not present
    if [ ! -f "$INPUT_ISO" ]; then
        wget -O "$INPUT_ISO" "https://releases.ubuntu.com/${UBUNTU_VERSION}/${ISO_NAME}"
    fi
fi

if [ ! -f "$INPUT_ISO" ]; then
    echo "ERROR: Input ISO not found: $INPUT_ISO"
    exit 1
fi

echo "==================================="
echo "Ubuntu Autoinstall ISO Remaster"
echo "==================================="
echo "Input ISO:     $INPUT_ISO"
echo "Output ISO:    $OUTPUT_ISO"
echo "Architecture:  $ARCH"
echo "Work dir:      $WORK_DIR"
echo "==================================="
echo ""

# Cleanup function
cleanup() {
    echo "Cleaning up..."
    sudo umount "$ISO_MOUNT" 2>/dev/null || true
    if [ $KEEP_WORK -eq 0 ]; then
        rm -rf "$WORK_DIR"
    else
        echo "Keeping work directory: $WORK_DIR"
    fi
}

trap cleanup EXIT

# Create working directories
rm -rf "$WORK_DIR"
mkdir -p "$ISO_EXTRACT" "$ISO_MOUNT" "$ISO_BUILD"

# Extract ISO
echo "Extracting ISO..."
sudo mount -o loop "$INPUT_ISO" "$ISO_MOUNT"
rsync -a "$ISO_MOUNT/" "$ISO_EXTRACT/"
sudo umount "$ISO_MOUNT"

# Copy ISO contents to build directory
echo "Preparing build directory..."
cp -rT "$ISO_EXTRACT" "$ISO_BUILD"

# Create autoinstall directory in ISO
mkdir -p "$ISO_BUILD/autoinstall"

# Copy autoinstall files
echo "Adding autoinstall configuration..."
cp "$SCRIPT_DIR/user-data" "$ISO_BUILD/autoinstall/"
cp "$SCRIPT_DIR/meta-data" "$ISO_BUILD/autoinstall/"

# Download and embed Tailscale package
echo "Downloading Tailscale package..."
mkdir -p "$ISO_BUILD/pool/extras"
if [ "$ARCH" = "amd64" ]; then
    TAILSCALE_DEB="tailscale_latest_amd64.deb"
    wget -O "$ISO_BUILD/pool/extras/tailscale.deb" "https://pkgs.tailscale.com/stable/ubuntu/pool/tailscale_amd64.deb" || echo "WARNING: Failed to download Tailscale package"
else
    TAILSCALE_DEB="tailscale_latest_arm64.deb"
    wget -O "$ISO_BUILD/pool/extras/tailscale.deb" "https://pkgs.tailscale.com/stable/ubuntu/pool/tailscale_arm64.deb" || echo "WARNING: Failed to download Tailscale package"
fi

# Modify GRUB configuration for autoinstall
echo "Modifying GRUB configuration..."
GRUB_CFG="$ISO_BUILD/boot/grub/grub.cfg"

if [ -f "$GRUB_CFG" ]; then
    # Backup original
    cp "$GRUB_CFG" "$GRUB_CFG.orig"

    # Add autoinstall parameter to default menu entry
    sed -i 's/---/ autoinstall ds=nocloud\;s=\/cdrom\/autoinstall\/ ---/' "$GRUB_CFG"

    # Set timeout to 1 second for faster boot
    sed -i 's/set timeout=.*/set timeout=1/' "$GRUB_CFG"
else
    echo "WARNING: GRUB config not found at expected location"
fi

# Also modify isolinux config if present
ISOLINUX_CFG="$ISO_BUILD/isolinux/txt.cfg"
if [ -f "$ISOLINUX_CFG" ]; then
    cp "$ISOLINUX_CFG" "$ISOLINUX_CFG.orig"
    sed -i 's/---/ autoinstall ds=nocloud\;s=\/cdrom\/autoinstall\/ ---/' "$ISOLINUX_CFG"
fi

# Update MD5 checksums
echo "Updating checksums..."
cd "$ISO_BUILD"
find . -type f -not -path './isolinux/*' -not -path './boot/grub/*' -print0 | xargs -0 md5sum > md5sum.txt

# Repack ISO
echo "Creating new ISO..."
cd "$ISO_BUILD"

if [ "$ARCH" = "amd64" ]; then
    xorriso -as mkisofs \
        -r -V "Ubuntu ${UBUNTU_VERSION} Autoinstall" \
        -J -joliet-long -l \
        -b isolinux/isolinux.bin \
        -c isolinux/boot.cat \
        -no-emul-boot -boot-load-size 4 -boot-info-table \
        -eltorito-alt-boot \
        -e boot/grub/efi.img \
        -no-emul-boot \
        -isohybrid-gpt-basdat \
        -isohybrid-apm-hfsplus \
        -isohybrid-mbr /usr/lib/ISOLINUX/isohdpfx.bin \
        -o "$OUTPUT_ISO" \
        .
else
    # ARM64 ISO creation
    xorriso -as mkisofs \
        -r -V "Ubuntu ${UBUNTU_VERSION} Autoinstall ARM64" \
        -J -joliet-long -l \
        -e boot/grub/efi.img \
        -no-emul-boot \
        -o "$OUTPUT_ISO" \
        .
fi

echo ""
echo "==================================="
echo "ISO created successfully!"
echo "==================================="
echo "Output: $OUTPUT_ISO"
echo "Size: $(du -h "$OUTPUT_ISO" | cut -f1)"
echo ""
echo "To use:"
echo "  1. Write to USB: sudo dd if=$OUTPUT_ISO of=/dev/sdX bs=4M status=progress"
echo "  2. Or burn to DVD"
echo "  3. Boot and installation will proceed automatically"
echo ""
echo "Default login after install:"
echo "  Username: ubuntu"
echo "  Password: ubuntu (change immediately!)"
echo "==================================="
