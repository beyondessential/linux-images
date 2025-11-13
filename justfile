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
    cd {{packer_dir}} && packer build -only='qemu.bare-metal' -var-file=amd64.pkrvars.hcl ubuntu-24.04.pkr.hcl

# Build only bare metal image for ARM64
build-bare-metal-arm64: (generate-autoinstall "arm64")
    @echo "Building bare metal image for ARM64..."
    @echo "NOTE: This will be slow on AMD64 host (uses emulation)"
    cd {{packer_dir}} && packer build -only='qemu.bare-metal' -var-file=arm64.pkrvars.hcl ubuntu-24.04.pkr.hcl

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
