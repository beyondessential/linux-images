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

create-iso-amd64: (generate-autoinstall "amd64")
    @echo "Creating AMD64 ISO..."
    cd {{autoinstall_dir}} && ./remaster-iso.sh --arch amd64 --user-data user-data-amd64

create-iso-arm64: (generate-autoinstall "arm64")
    @echo "Creating ARM64 ISO..."
    cd {{autoinstall_dir}} && ./remaster-iso.sh --arch arm64 --user-data user-data-arm64

qemu-amd64: create-iso-amd64
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Building bare metal image directly with QEMU for AMD64..."

    ISO_FILE=$(ls -1t {{autoinstall_dir}}/ubuntu-*-bes-server-amd64-*.iso | head -1)
    if [ -z "$ISO_FILE" ]; then
        echo "ERROR: No AMD64 ISO found in {{autoinstall_dir}}"
        exit 1
    fi
    echo "Using ISO: $ISO_FILE"

    OUTPUT_DIR="{{output_dir}}/qemu-amd64"
    mkdir -p "$OUTPUT_DIR"

    TIMESTAMP=$(date +%Y%m%d%H%M%S)
    DISK_IMAGE="$OUTPUT_DIR/ubuntu-24.04-amd64-${TIMESTAMP}.raw"
    OVMF_VARS="$OUTPUT_DIR/OVMF_VARS-${TIMESTAMP}.fd"

    # Find OVMF firmware files
    OVMF_CODE=""
    for path in /usr/share/edk2/x64/OVMF_CODE.4m.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2/ovmf/OVMF_CODE.fd /usr/share/ovmf/x64/OVMF_CODE.fd; do
        if [ -f "$path" ]; then
            OVMF_CODE="$path"
            break
        fi
    done

    if [ -z "$OVMF_CODE" ]; then
        echo "ERROR: OVMF_CODE.fd not found. Install ovmf package."
        exit 1
    fi

    OVMF_VARS_TEMPLATE=""
    for path in /usr/share/edk2/x64/OVMF_VARS.4m.fd /usr/share/OVMF/OVMF_VARS.fd /usr/share/edk2/ovmf/OVMF_VARS.fd /usr/share/ovmf/x64/OVMF_VARS.fd; do
        if [ -f "$path" ]; then
            OVMF_VARS_TEMPLATE="$path"
            break
        fi
    done

    if [ -z "$OVMF_VARS_TEMPLATE" ]; then
        echo "ERROR: OVMF_VARS.fd not found. Install ovmf package."
        exit 1
    fi

    cp "$OVMF_VARS_TEMPLATE" "$OVMF_VARS"

    qemu-img create -f raw "$DISK_IMAGE" 8G

    echo ""
    echo "This will run the automated installer. Wait for it to complete fully,"
    echo "then shut down the machine from the menu."

    qemu-system-x86_64 \
        -enable-kvm \
        -m 4096 \
        -smp 2 \
        -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
        -drive if=pflash,format=raw,file="$OVMF_VARS" \
        -drive file="$DISK_IMAGE",format=raw,if=virtio \
        -cdrom "$ISO_FILE" \
        -boot d

    echo "Installation complete! Disk image at: $DISK_IMAGE"
    echo "Generating checksum..."
    sha256sum "$DISK_IMAGE" > "${DISK_IMAGE}.sha256"
    echo "Done!"

qemu-arm64: create-iso-arm64
    #!/usr/bin/env bash
    set -euo pipefail

    ISO_FILE=$(ls -1t {{autoinstall_dir}}/ubuntu-*-bes-server-arm64-*.iso | head -1)
    if [ -z "$ISO_FILE" ]; then
        echo "ERROR: No ARM64 ISO found in {{autoinstall_dir}}"
        exit 1
    fi
    echo "Using ISO: $ISO_FILE"

    OUTPUT_DIR="{{output_dir}}/qemu-arm64"
    mkdir -p "$OUTPUT_DIR"

    TIMESTAMP=$(date +%Y%m%d%H%M%S)
    DISK_IMAGE="$OUTPUT_DIR/ubuntu-24.04-arm64-${TIMESTAMP}.raw"
    AAVMF_VARS="$OUTPUT_DIR/AAVMF_VARS-${TIMESTAMP}.fd"

    # Find AAVMF firmware files for ARM64
    AAVMF_CODE=""
    for path in /usr/share/edk2/aarch64/QEMU_CODE.fd /usr/share/AAVMF/AAVMF_CODE.fd /usr/share/qemu-efi-aarch64/QEMU_EFI.fd /usr/share/edk2/aarch64/QEMU_EFI-pflash.raw; do
        if [ -f "$path" ]; then
            AAVMF_CODE="$path"
            break
        fi
    done

    if [ -z "$AAVMF_CODE" ]; then
        echo "ERROR: AAVMF/QEMU_EFI firmware not found. Install qemu-efi-aarch64 package."
        exit 1
    fi

    AAVMF_VARS_TEMPLATE=""
    for path in /usr/share/edk2/aarch64/QEMU_VARS.fd /usr/share/AAVMF/AAVMF_VARS.fd /usr/share/qemu-efi-aarch64/QEMU_VARS.fd /usr/share/edk2/aarch64/vars-template-pflash.raw; do
        if [ -f "$path" ]; then
            AAVMF_VARS_TEMPLATE="$path"
            break
        fi
    done

    if [ -z "$AAVMF_VARS_TEMPLATE" ]; then
        echo "ERROR: AAVMF_VARS firmware not found. Install qemu-efi-aarch64 package."
        exit 1
    fi

    cp "$AAVMF_VARS_TEMPLATE" "$AAVMF_VARS"

    qemu-img create -f raw "$DISK_IMAGE" 8G

    echo ""
    echo "This will run the automated installer. Wait for it to complete fully,"
    echo "then shut down the machine from the menu."

    qemu-system-aarch64 \
        -machine virt \
        -cpu cortex-a57 \
        -m 4096 \
        -smp 2 \
        -drive if=pflash,format=raw,readonly=on,file="$AAVMF_CODE" \
        -drive if=pflash,format=raw,file="$AAVMF_VARS" \
        -drive file="$DISK_IMAGE",format=raw,if=virtio \
        -cdrom "$ISO_FILE" \
        -boot d

    echo "Installation complete! Disk image at: $DISK_IMAGE"
    echo "Generating checksum..."
    sha256sum "$DISK_IMAGE" > "${DISK_IMAGE}.sha256"
    echo "Done!"
