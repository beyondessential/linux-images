# Ubuntu 24.04 Custom Image Builder with BTRFS+LUKS
#
# Workflow:
# 1. Generate autoinstall config: just generate-autoinstall amd64
# 2. Create custom ISO: just create-iso-amd64
# 3. Build bare metal image: just build-bare-metal-amd64
# 4. Import to AWS: just import-aws-amd64
# 5. Register AMI: cd scripts && ./register-ami.sh <import-task-id>

packer_dir := "packer"
output_dir := "output"
autoinstall_dir := "iso"

# Show available recipes
default:
    @just --list

# Generate autoinstall user-data from scripts
generate-autoinstall arch="amd64":
    @echo "Generating autoinstall user-data for {{arch}}..."
    cd {{autoinstall_dir}} && node generate-user-data.js user-data-{{arch}} {{arch}}
    @echo "Generated iso/user-data-{{arch}} for {{arch}}"

# Initialize and install dependencies
init:
    @echo "Checking dependencies..."
    @command -v packer >/dev/null 2>&1 || { echo "ERROR: packer is not installed"; exit 1; }
    @command -v qemu-system-x86_64 >/dev/null 2>&1 || echo "WARNING: qemu-system-x86_64 not found (needed for bare metal builds)"
    @command -v qemu-system-aarch64 >/dev/null 2>&1 || echo "WARNING: qemu-system-aarch64 not found (needed for ARM64 bare metal builds)"
    @echo "Installing Packer plugins..."
    cd {{packer_dir}} && packer init ubuntu-24.04.pkr.hcl
    @echo "Dependencies ready!"

# Validate Packer configurations
validate:
    @echo "Validating Packer configuration for AMD64..."
    cd {{packer_dir}} && packer validate -var-file=amd64.pkrvars.hcl ubuntu-24.04.pkr.hcl
    @echo "Validating Packer configuration for ARM64..."
    cd {{packer_dir}} && packer validate -var-file=arm64.pkrvars.hcl ubuntu-24.04.pkr.hcl
    @echo "All configurations are valid!"

# === Building Images ===

# Build bare metal image for AMD64
build-amd64: (create-iso-amd64)
    @echo "Building AMD64 bare metal image..."
    cd {{packer_dir}} && packer build -var-file=amd64.pkrvars.hcl ubuntu-24.04.pkr.hcl

# Build bare metal image for ARM64
build-arm64: (create-iso-arm64)
    @echo "Building ARM64 bare metal image..."
    @echo "NOTE: ARM64 bare metal build will be slow on AMD64 host (uses emulation)"
    cd {{packer_dir}} && packer build -var-file=arm64.pkrvars.hcl ubuntu-24.04.pkr.hcl

# Build all bare metal images (AMD64 + ARM64)
build-all: (create-iso-amd64) (create-iso-arm64)
    @echo "Building all bare metal images (AMD64 + ARM64)..."
    @echo "This will take a long time..."
    just build-amd64
    just build-arm64

# Build only bare metal image for AMD64
build-bare-metal-amd64: (generate-autoinstall "amd64")
    @echo "Building bare metal image for AMD64..."
    cd {{packer_dir}} && packer build -only='ubuntu-bare-metal.qemu.bare-metal' -var-file=amd64.pkrvars.hcl -var="custom_iso_path=$(ls -1 ../iso/ubuntu-*-bes-server-amd64-*.iso | head -1)" ubuntu-24.04.pkr.hcl

# Build only bare metal image for ARM64
build-bare-metal-arm64: (generate-autoinstall "arm64")
    @echo "Building bare metal image for ARM64..."
    @echo "NOTE: This will be slow on AMD64 host (uses emulation)"
    cd {{packer_dir}} && packer build -only='ubuntu-bare-metal.qemu.bare-metal' -var-file=arm64.pkrvars.hcl -var="custom_iso_path=$(ls -1 ../iso/ubuntu-*-bes-server-arm64-*.iso | head -1)" ubuntu-24.04.pkr.hcl

# Build bare metal image directly with QEMU (no Packer) for AMD64
qemu-direct-amd64: create-iso-amd64
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Building bare metal image directly with QEMU for AMD64..."

    ISO_FILE=$(ls -1 {{autoinstall_dir}}/ubuntu-*-bes-server-amd64-*.iso | head -1)
    if [ -z "$ISO_FILE" ]; then
        echo "ERROR: No AMD64 ISO found in {{autoinstall_dir}}"
        exit 1
    fi
    echo "Using ISO: $ISO_FILE"

    OUTPUT_DIR="{{output_dir}}/qemu-direct-amd64"
    mkdir -p "$OUTPUT_DIR"

    TIMESTAMP=$(date +%Y%m%d%H%M%S)
    DISK_IMAGE="$OUTPUT_DIR/ubuntu-24.04-amd64-${TIMESTAMP}.raw"

    echo "Creating disk image: $DISK_IMAGE"
    qemu-img create -f raw "$DISK_IMAGE" 8G

    echo "Starting QEMU installation..."
    echo "This will run the automated installer. Wait for it to complete and shut down."

    qemu-system-x86_64 \
        -enable-kvm \
        -m 4096 \
        -smp 2 \
        -drive file="$DISK_IMAGE",format=raw,if=virtio \
        -cdrom "$ISO_FILE" \
        -boot d

    echo "Installation complete! Disk image at: $DISK_IMAGE"
    echo "Generating checksum..."
    sha256sum "$DISK_IMAGE" > "${DISK_IMAGE}.sha256"
    echo "Done!"

# Build bare metal image directly with QEMU (no Packer) for ARM64
qemu-direct-arm64: create-iso-arm64
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Building bare metal image directly with QEMU for ARM64..."
    echo "NOTE: This will be slow on AMD64 host (uses emulation)"

    ISO_FILE=$(ls -1 {{autoinstall_dir}}/ubuntu-*-bes-server-arm64-*.iso | head -1)
    if [ -z "$ISO_FILE" ]; then
        echo "ERROR: No ARM64 ISO found in {{autoinstall_dir}}"
        exit 1
    fi
    echo "Using ISO: $ISO_FILE"

    OUTPUT_DIR="{{output_dir}}/qemu-direct-arm64"
    mkdir -p "$OUTPUT_DIR"

    TIMESTAMP=$(date +%Y%m%d%H%M%S)
    DISK_IMAGE="$OUTPUT_DIR/ubuntu-24.04-arm64-${TIMESTAMP}.raw"

    echo "Creating disk image: $DISK_IMAGE"
    qemu-img create -f raw "$DISK_IMAGE" 8G

    echo "Starting QEMU installation..."
    echo "This will run the automated installer. Wait for it to complete and shut down."

    qemu-system-aarch64 \
        -machine virt \
        -cpu cortex-a57 \
        -m 4096 \
        -smp 2 \
        -drive file="$DISK_IMAGE",format=raw,if=virtio \
        -cdrom "$ISO_FILE" \
        -boot d

    echo "Installation complete! Disk image at: $DISK_IMAGE"
    echo "Generating checksum..."
    sha256sum "$DISK_IMAGE" > "${DISK_IMAGE}.sha256"
    echo "Done!"

# === AWS Import ===

# Import bare metal image to AWS as AMI for AMD64
import-aws-amd64: build-bare-metal-amd64
    @echo "Importing bare metal AMD64 image to AWS..."
    @command -v aws >/dev/null 2>&1 || { echo "ERROR: AWS CLI not installed"; exit 1; }
    @echo "NOTE: This will upload the image to S3 and create an import task."
    @echo "This requires AdminAccess and may take 30+ minutes."
    @echo "Review the script carefully before proceeding."
    @read -p "Press enter to continue or Ctrl+C to cancel..."
    cd scripts && aws-sso exec -p _BES_Primary:AdminAccess -- ./import-to-aws.sh amd64

# Import bare metal image to AWS as AMI for ARM64
import-aws-arm64: build-bare-metal-arm64
    @echo "Importing bare metal ARM64 image to AWS..."
    @command -v aws >/dev/null 2>&1 || { echo "ERROR: AWS CLI not installed"; exit 1; }
    @echo "NOTE: This will upload the image to S3 and create an import task."
    @echo "This requires AdminAccess and may take 30+ minutes."
    @echo "Review the script carefully before proceeding."
    @read -p "Press enter to continue or Ctrl+C to cancel..."
    cd scripts && aws-sso exec -p _BES_Primary:AdminAccess -- ./import-to-aws.sh arm64

# === ISO Creation ===

# Create custom ISO with embedded autoinstall config
create-iso-amd64: (generate-autoinstall "amd64")
    @echo "Creating AMD64 ISO..."
    cd {{autoinstall_dir}} && ./remaster-iso.sh --arch amd64 --user-data user-data-amd64

# Create custom ISO with embedded autoinstall config
create-iso-arm64: (generate-autoinstall "arm64")
    @echo "Creating ARM64 ISO..."
    cd {{autoinstall_dir}} && ./remaster-iso.sh --arch arm64 --user-data user-data-arm64

# === Maintenance ===

# Remove build artifacts
clean:
    @echo "Cleaning build artifacts..."
    rm -rf {{output_dir}}
    rm -rf {{packer_dir}}/packer_cache
    rm -rf {{packer_dir}}/output-*
    @echo "Clean complete!"

# === Development ===

# Inspect AMD64 Packer configuration
dev-inspect-amd64:
    @echo "Inspecting AMD64 Packer configuration..."
    cd {{packer_dir}} && packer inspect ubuntu-24.04.pkr.hcl

# Format Packer HCL files
dev-format:
    @echo "Formatting Packer HCL files..."
    cd {{packer_dir}} && packer fmt .
