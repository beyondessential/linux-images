#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UBUNTU_VERSION="24.04.3"
WORK_DIR="${WORK_DIR:-/tmp/ubuntu-remaster}"
ORIGINAL_DIR="$(pwd)"

usage() {
  cat <<EOF
Usage: $0 [OPTIONS]

Create a custom Ubuntu ISO with embedded autoinstall configuration

OPTIONS:
    -a, --arch ARCH          Architecture: amd64 or arm64 (default: amd64)
    -i, --input ISO          Input ISO file (if not provided, will download)
    -o, --output ISO         Output ISO file (default: ubuntu-24.04-bes-server-ARCH.iso)
    -u, --user-data FILE     User data file (default: user-data-ARCH)
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
    - docker
    - xorriso or genisoimage
    - wget or curl

EOF
  exit 0
}

# Parse arguments
ARCH="amd64"
INPUT_ISO=""
OUTPUT_ISO=""
USER_DATA=""
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
    -u|--user-data)
      USER_DATA="$2"
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
BUILD_DATE="$(date +%Y%m%d)"
BUILD_DATE_DISPLAY="$(date +%Y-%m-%d)"
GIT_SHORT_HASH="$(git -C "$SCRIPT_DIR" rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
OUTPUT_ISO="${OUTPUT_ISO:-ubuntu-${UBUNTU_VERSION}-bes-server-${ARCH}-${BUILD_DATE}.iso}"
USER_DATA="${USER_DATA:-user-data-${ARCH}}"
ISO_EXTRACT="$WORK_DIR/extract"
ISO_BUILD="$WORK_DIR/build"

# Validate architecture
if [[ "$ARCH" != "amd64" && "$ARCH" != "arm64" ]]; then
  echo "ERROR: Architecture must be amd64 or arm64"
  exit 1
fi

# Check for docker
if ! command -v docker &> /dev/null; then
  echo "ERROR: docker not found"
  echo "Install with: sudo apt-get install docker.io"
  exit 1
fi

echo "Using docker for ISO extraction"

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
  CHECKSUM_FILE="/tmp/SHA256SUMS-${UBUNTU_VERSION}"

  if [ "$ARCH" = "amd64" ]; then
    ISO_URL="https://releases.ubuntu.com/${UBUNTU_VERSION}/${ISO_NAME}"
    SUM_URL="https://releases.ubuntu.com/${UBUNTU_VERSION}/SHA256SUMS"
  else
    ISO_URL="https://cdimage.ubuntu.com/releases/${UBUNTU_VERSION}/release/${ISO_NAME}"
    SUM_URL="https://cdimage.ubuntu.com/releases/${UBUNTU_VERSION}/release/SHA256SUMS"
  fi

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
    echo "from $ISO_URL"
    wget -O "$INPUT_ISO" "$ISO_URL"
  fi

  # Download and verify checksum
  echo "Downloading checksums..."
  echo "from $SUM_URL"
  wget -q -O "$CHECKSUM_FILE" "$SUM_URL"

  echo "Verifying ISO checksum..."
  EXPECTED_CHECKSUM=$(grep "${ISO_NAME}" "$CHECKSUM_FILE" | awk '{print $1}')

  if [ -z "$EXPECTED_CHECKSUM" ]; then
    echo "ERROR: Could not find checksum for ${ISO_NAME} in SHA256SUMS"
    rm -f "$INPUT_ISO" "$CHECKSUM_FILE"
    exit 1
  fi

  ACTUAL_CHECKSUM=$(sha256sum "$INPUT_ISO" | awk '{print $1}')

  if [ "$EXPECTED_CHECKSUM" != "$ACTUAL_CHECKSUM" ]; then
    echo "ERROR: Checksum verification failed!"
    echo "Expected: $EXPECTED_CHECKSUM"
    echo "Got:      $ACTUAL_CHECKSUM"
    rm -f "$INPUT_ISO" "$CHECKSUM_FILE"
    exit 1
  fi

  echo "Checksum verification passed"
  rm -f "$CHECKSUM_FILE"
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
  if [ $KEEP_WORK -eq 0 ]; then
    rm -rf "$WORK_DIR"
  else
    echo "Keeping work directory: $WORK_DIR"
  fi
}

trap cleanup EXIT

# Create working directories
rm -rf "$WORK_DIR"
mkdir -p "$ISO_EXTRACT" "$ISO_BUILD"

# Extract ISO using container (no sudo needed)
echo "Extracting ISO..."
INPUT_ISO_ABS="$(readlink -f "$INPUT_ISO")"
ISO_EXTRACT_ABS="$(readlink -f "$ISO_EXTRACT")"

docker run --rm \
  -v "$INPUT_ISO_ABS:/input.iso:ro" \
  -v "$ISO_EXTRACT_ABS:/output:rw" \
  ubuntu:24.04 \
  bash -c "apt-get update -qq && apt-get install -y -qq xorriso > /dev/null 2>&1 && \
          xorriso -abort_on NEVER -osirrox on -indev /input.iso -extract / /output && \
          xorriso -abort_on NEVER -indev /input.iso -osirrox on -extract_boot_images /output/ && \
          chown -R $(id -u):$(id -g) /output && \
          chmod -R u+w /output"

# Copy ISO contents to build directory
echo "Preparing build directory..."
cp -rT "$ISO_EXTRACT" "$ISO_BUILD"

# Move EFI boot image to boot directory
if [ -f "$ISO_EXTRACT/eltorito_img2_uefi.img" ]; then
  mkdir -p "$ISO_BUILD/boot/grub"
  mv "$ISO_EXTRACT/eltorito_img2_uefi.img" "$ISO_BUILD/boot/grub/efi.img"
  echo "Preserved EFI boot image"
fi

# Copy autoinstall configuration to root of ISO
# Subiquity looks for /autoinstall.yaml when 'autoinstall' kernel parameter is present
echo "Adding autoinstall configuration..."
if [ ! -f "$SCRIPT_DIR/$USER_DATA" ]; then
  echo "ERROR: User data file not found: $SCRIPT_DIR/$USER_DATA"
  exit 1
fi
cp "$SCRIPT_DIR/$USER_DATA" "$ISO_BUILD/autoinstall.yaml"

# Modify GRUB configuration for autoinstall
echo "Modifying GRUB configuration..."
GRUB_CFG="$ISO_BUILD/boot/grub/grub.cfg"

if [ -f "$GRUB_CFG" ]; then
  mv "$GRUB_CFG" "$GRUB_CFG.orig"
  cat > "$GRUB_CFG" <<EOF
set timeout=5

loadfont unicode

set menu_color_normal=white/black
set menu_color_highlight=black/light-gray

menuentry "Auto install Ubuntu BES Server" {
  set gfxpayload=keep
  linux   /casper/vmlinuz autoinstall ---
  initrd  /casper/initrd
}
menuentry "Rescue (live)" {
  set gfxpayload=keep
  linux   /casper/vmlinuz boot=casper interactive systemd.mask=snap.subiquity.subiquity-service.service ---
  initrd  /casper/initrd
}
menuentry "--- Built by BES at $BUILD_DATE_DISPLAY from $GIT_SHORT_HASH ---" {
  reboot
}

grub_platform
if [ "\$grub_platform" = "efi" ]; then
menuentry 'Boot from next volume' {
  exit 1
}
menuentry 'UEFI Firmware Settings' {
  fwsetup
}
else
menuentry 'Test memory' {
  linux16 /boot/memtest86+x64.bin
}
fi
EOF
else
  echo "WARNING: GRUB config not found at expected location"
fi

# Update checksums
echo "Updating checksums..."
cd "$ISO_BUILD"
find . -type f -not -path './boot/grub/*' -not -path './EFI/*' -print0 | xargs -0 md5sum > md5sum.txt

# Repack ISO
echo "Creating new ISO..."
cd "$ISO_BUILD"

# Create ISO in temp location first
TEMP_ISO="$WORK_DIR/$(basename "$OUTPUT_ISO")"

if [ "$ARCH" = "amd64" ]; then
  xorriso -as mkisofs \
    -r -V "Ubuntu ${UBUNTU_VERSION} BES Server AMD64" \
    -J -joliet-long -l \
    -b boot/grub/i386-pc/eltorito.img \
    -c boot.catalog \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    --grub2-boot-info --grub2-mbr /usr/lib/grub/i386-pc/boot_hybrid.img \
    -eltorito-alt-boot \
    -e boot/grub/efi.img \
    -no-emul-boot \
    -append_partition 2 0xef boot/grub/efi.img \
    -appended_part_as_gpt \
    -o "$TEMP_ISO" \
    .
else
  xorriso -as mkisofs \
    -r -V "Ubuntu ${UBUNTU_VERSION} BES Server ARM64" \
    -J -joliet-long -l \
    -e boot/grub/efi.img \
    -no-emul-boot \
    -append_partition 2 0xef boot/grub/efi.img \
    -appended_part_as_gpt \
    -o "$TEMP_ISO" \
    .
fi

# Move ISO to original directory
cd "$ORIGINAL_DIR"
mv "$TEMP_ISO" "$OUTPUT_ISO"

# Generate SHA256 checksum for the ISO
echo "Generating SHA256 checksum for ISO..."
sha256sum "$OUTPUT_ISO" > "$OUTPUT_ISO.sha256"

echo ""
echo "==================================="
echo "ISO created successfully!"
echo "==================================="
echo "Output: $OUTPUT_ISO"
echo "Size: $(du -h "$OUTPUT_ISO" | cut -f1)"
echo "SHA256: $(cat "$OUTPUT_ISO.sha256" | awk '{print $1}')"
echo ""
echo "To use:"
echo "  1. Write to USB: dd if=$OUTPUT_ISO of=/dev/sdX bs=4M status=progress"
echo "  2. Boot and installation will proceed automatically"
echo ""
echo "Default login after install:"
echo "  Username: ubuntu"
echo "  Password: bes"
echo "==================================="
