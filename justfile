# Ubuntu 24.04 Custom Image Builder

packer_dir := "packer"
output_dir := "output"
autoinstall_dir := "autoinstall"

# Show available recipes
default:
    @just --list

# Generate autoinstall user-data from scripts
generate-autoinstall:
    @echo "Generating autoinstall user-data..."
    cd {{autoinstall_dir}} && node generate-user-data.js user-data
    @echo "Generated autoinstall/user-data"

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

# Build both bare metal and AWS images for AMD64
build-amd64:
    @echo "Building AMD64 images (bare metal + AWS)..."
    cd {{packer_dir}} && packer build -var-file=amd64.pkrvars.hcl ubuntu-24.04.pkr.hcl

# Build both bare metal and AWS images for ARM64
build-arm64:
    @echo "Building ARM64 images (bare metal + AWS)..."
    @echo "NOTE: ARM64 bare metal build will be slow on AMD64 host (uses emulation)"
    cd {{packer_dir}} && packer build -var-file=arm64.pkrvars.hcl ubuntu-24.04.pkr.hcl

# Build all images (AMD64 + ARM64)
build-all:
    @echo "Building all images (AMD64 + ARM64)..."
    @echo "This will take a long time..."
    just build-amd64
    just build-arm64

# Build only bare metal image for AMD64
build-bare-metal-amd64: generate-autoinstall
    @echo "Building bare metal image for AMD64..."
    cd {{packer_dir}} && packer build -only='qemu.bare-metal' -var-file=amd64.pkrvars.hcl ubuntu-24.04.pkr.hcl

# Build only bare metal image for ARM64
build-bare-metal-arm64: generate-autoinstall
    @echo "Building bare metal image for ARM64..."
    @echo "NOTE: This will be slow on AMD64 host (uses emulation)"
    cd {{packer_dir}} && packer build -only='qemu.bare-metal' -var-file=arm64.pkrvars.hcl ubuntu-24.04.pkr.hcl

# Build only AWS AMI for AMD64
build-aws-amd64:
    @echo "Building AWS AMI for AMD64..."
    @command -v aws >/dev/null 2>&1 || { echo "ERROR: AWS CLI not installed"; exit 1; }
    @echo "NOTE: This will create resources in AWS. Ensure you have appropriate credentials."
    cd {{packer_dir}} && aws-sso exec -p _BES_Primary:ReadAccess -- packer build -only='amazon-ebs.aws' -var-file=amd64.pkrvars.hcl ubuntu-24.04.pkr.hcl

# Build only AWS AMI for ARM64
build-aws-arm64:
    @echo "Building AWS AMI for ARM64..."
    @command -v aws >/dev/null 2>&1 || { echo "ERROR: AWS CLI not installed"; exit 1; }
    @echo "NOTE: This will create resources in AWS. Ensure you have appropriate credentials."
    cd {{packer_dir}} && aws-sso exec -p _BES_Primary:ReadAccess -- packer build -only='amazon-ebs.aws' -var-file=arm64.pkrvars.hcl ubuntu-24.04.pkr.hcl

# Create custom ISO with embedded autoinstall config
create-iso-amd64: generate-autoinstall
    @echo "Creating AMD64 autoinstall ISO..."
    cd {{autoinstall_dir}} && ./remaster-iso.sh --arch amd64

# Create custom ISO with embedded autoinstall config
create-iso-arm64: generate-autoinstall
    @echo "Creating ARM64 autoinstall ISO..."
    cd {{autoinstall_dir}} && ./remaster-iso.sh --arch arm64

# Remove build artifacts
clean:
    @echo "Cleaning build artifacts..."
    rm -rf {{output_dir}}
    rm -rf {{packer_dir}}/packer_cache
    rm -rf {{packer_dir}}/output-*
    @echo "Clean complete!"

# Inspect AMD64 Packer configuration
dev-inspect-amd64:
    @echo "Inspecting AMD64 Packer configuration..."
    cd {{packer_dir}} && packer inspect ubuntu-24.04.pkr.hcl

# Format Packer HCL files
dev-format:
    @echo "Formatting Packer HCL files..."
    cd {{packer_dir}} && packer fmt .
