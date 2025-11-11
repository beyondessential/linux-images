#!/bin/bash
set -euo pipefail

# Test script for validating built images
# Can test bare metal images locally with QEMU before deployment

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${SCRIPT_DIR}/output"

usage() {
    cat <<EOF
Usage: $0 [OPTIONS] IMAGE_FILE

Test a built disk image using QEMU

OPTIONS:
    -a, --arch ARCH      Architecture: amd64 or arm64 (default: auto-detect)
    -m, --memory SIZE    Memory size in MB (default: 2048)
    -c, --cpus COUNT     Number of CPUs (default: 2)
    -p, --port PORT      SSH port forwarding (default: 2222)
    --vnc                Enable VNC display (default: headless)
    --decompress         Decompress .zst file first
    -h, --help           Show this help

EXAMPLES:
    # Test AMD64 image
    $0 output/bare-metal-amd64/ubuntu-24.04-amd64-*.raw

    # Test ARM64 image with VNC display
    $0 --arch arm64 --vnc output/bare-metal-arm64/ubuntu-24.04-arm64-*.raw

    # Test compressed image (will decompress first)
    $0 --decompress output/bare-metal-amd64/ubuntu-24.04-amd64-*.raw.zst

    # Custom resources
    $0 -m 4096 -c 4 -p 2222 myimage.raw

EOF
    exit 0
}

# Default values
ARCH=""
MEMORY=2048
CPUS=2
SSH_PORT=2222
VNC_DISPLAY=""
DECOMPRESS=0
IMAGE_FILE=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--arch)
            ARCH="$2"
            shift 2
            ;;
        -m|--memory)
            MEMORY="$2"
            shift 2
            ;;
        -c|--cpus)
            CPUS="$2"
            shift 2
            ;;
        -p|--port)
            SSH_PORT="$2"
            shift 2
            ;;
        --vnc)
            VNC_DISPLAY="-vnc :0"
            shift
            ;;
        --decompress)
            DECOMPRESS=1
            shift
            ;;
        -h|--help)
            usage
            ;;
        -*)
            echo "Unknown option: $1"
            usage
            ;;
        *)
            IMAGE_FILE="$1"
            shift
            ;;
    esac
done

if [ -z "$IMAGE_FILE" ]; then
    echo "ERROR: No image file specified"
    usage
fi

if [ ! -f "$IMAGE_FILE" ]; then
    echo "ERROR: Image file not found: $IMAGE_FILE"
    exit 1
fi

# Decompress if needed
if [ "$DECOMPRESS" -eq 1 ]; then
    if [[ "$IMAGE_FILE" == *.zst ]]; then
        echo "Decompressing $IMAGE_FILE..."
        RAW_FILE="${IMAGE_FILE%.zst}"
        if [ -f "$RAW_FILE" ]; then
            echo "WARNING: $RAW_FILE already exists, skipping decompression"
        else
            zstd -d "$IMAGE_FILE" -o "$RAW_FILE"
        fi
        IMAGE_FILE="$RAW_FILE"
    else
        echo "WARNING: --decompress specified but file is not .zst"
    fi
fi

# Auto-detect architecture if not specified
if [ -z "$ARCH" ]; then
    if [[ "$IMAGE_FILE" == *"amd64"* ]] || [[ "$IMAGE_FILE" == *"x86_64"* ]]; then
        ARCH="amd64"
    elif [[ "$IMAGE_FILE" == *"arm64"* ]] || [[ "$IMAGE_FILE" == *"aarch64"* ]]; then
        ARCH="arm64"
    else
        echo "ERROR: Could not auto-detect architecture. Please specify with --arch"
        exit 1
    fi
fi

# Set QEMU binary and machine type based on architecture
case "$ARCH" in
    amd64|x86_64)
        QEMU_BIN="qemu-system-x86_64"
        MACHINE_TYPE="q35"
        ACCEL_FLAGS="-enable-kvm"
        CPU_TYPE="host"
        # Check if KVM is available
        if [ ! -e /dev/kvm ]; then
            echo "WARNING: KVM not available, using emulation (will be slow)"
            ACCEL_FLAGS=""
            CPU_TYPE="qemu64"
        fi
        ;;
    arm64|aarch64)
        QEMU_BIN="qemu-system-aarch64"
        MACHINE_TYPE="virt"
        CPU_TYPE="cortex-a72"
        ACCEL_FLAGS=""
        # Check if we're on ARM64 host
        if [ "$(uname -m)" = "aarch64" ] && [ -e /dev/kvm ]; then
            ACCEL_FLAGS="-enable-kvm"
            CPU_TYPE="host"
        else
            echo "WARNING: Not on ARM64 host or KVM unavailable, using emulation (will be slow)"
        fi
        ;;
    *)
        echo "ERROR: Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

# Check if QEMU is installed
if ! command -v "$QEMU_BIN" &> /dev/null; then
    echo "ERROR: $QEMU_BIN not found"
    echo "Install with: sudo apt-get install qemu-system-x86 qemu-system-arm"
    exit 1
fi

# Set display mode
if [ -z "$VNC_DISPLAY" ]; then
    DISPLAY_FLAGS="-nographic"
else
    DISPLAY_FLAGS="$VNC_DISPLAY"
    echo "VNC display available at localhost:5900"
fi

echo "==================================="
echo "Testing Image: $IMAGE_FILE"
echo "Architecture: $ARCH"
echo "Memory: ${MEMORY}MB"
echo "CPUs: $CPUS"
echo "SSH Port: $SSH_PORT (localhost:$SSH_PORT -> guest:22)"
echo "QEMU Binary: $QEMU_BIN"
echo "==================================="
echo ""
echo "To connect via SSH (once booted):"
echo "  ssh -p $SSH_PORT ubuntu@localhost"
echo ""
echo "Default credentials:"
echo "  Username: ubuntu"
echo "  Password: ubuntu (or use SSH key)"
echo ""
echo "Press Ctrl+C to stop the VM"
echo "==================================="
echo ""

# Build QEMU command
QEMU_CMD=(
    "$QEMU_BIN"
    -machine "$MACHINE_TYPE"
    -cpu "$CPU_TYPE"
    -m "$MEMORY"
    -smp "$CPUS"
    -drive "file=$IMAGE_FILE,format=raw,if=virtio"
    -netdev "user,id=net0,hostfwd=tcp::${SSH_PORT}-:22"
    -device "virtio-net-pci,netdev=net0"
    $ACCEL_FLAGS
    $DISPLAY_FLAGS
)

# Add EFI for ARM64
if [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
    QEMU_CMD+=(-bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd)
fi

# Run QEMU
echo "Starting QEMU..."
"${QEMU_CMD[@]}"
