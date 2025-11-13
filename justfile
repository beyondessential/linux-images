# Ubuntu 24.04 Custom Image Builder with BTRFS+LUKS
#
# Workflow:
# 1. Generate autoinstall config: just generate-autoinstall amd64
# 2. Create custom ISO: just create-iso-amd64
# 3. Build bare metal image: just build-bare-metal-amd64
# 4. Import to AWS: just import-aws-amd64
# 5. Register AMI: cd scripts && ./register-ami.sh <import-task-id>

autoinstall_dir := "iso"
work_dir := "working"
output_dir := "output"

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

prepare-firmware-amd64:
    #!/usr/bin/env bash
    set -euo pipefail

    WORK_DIR="{{work_dir}}/qemu-amd64"
    mkdir -p "$WORK_DIR"

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

    ln -sf "$OVMF_CODE" "$WORK_DIR/OVMF_CODE.fd"
    cp "$OVMF_VARS_TEMPLATE" "$WORK_DIR/OVMF_VARS.fd"
    echo "Prepared firmware in $WORK_DIR"

prepare-iso-amd64: create-iso-amd64
    #!/usr/bin/env bash
    set -euo pipefail

    WORK_DIR="{{work_dir}}/qemu-amd64"
    mkdir -p "$WORK_DIR"

    ISO_FILE=$(ls -1t {{autoinstall_dir}}/ubuntu-*-bes-server-amd64-*.iso | head -1)
    if [ -z "$ISO_FILE" ]; then
        echo "ERROR: No AMD64 ISO found in {{autoinstall_dir}}"
        exit 1
    fi

    ln -sf "$(realpath "$ISO_FILE")" "$WORK_DIR/installer.iso"
    echo "Prepared ISO: $ISO_FILE"

qemu-amd64: prepare-firmware-amd64 prepare-iso-amd64
    #!/usr/bin/env bash
    set -euo pipefail

    WORK_DIR="{{work_dir}}/qemu-amd64"
    OUTPUT_DIR="{{output_dir}}/qemu-amd64"
    mkdir -p "$OUTPUT_DIR"

    TIMESTAMP=$(date +%Y%m%d%H%M%S)
    DISK_IMAGE="$WORK_DIR/ubuntu-24.04-amd64-${TIMESTAMP}.raw"

    qemu-img create -f raw "$DISK_IMAGE" 8G

    echo ""
    echo "This will run the automated installer. Wait for it to complete fully,"
    echo "then shut down the machine from the menu."

    qemu-system-x86_64 \
        -enable-kvm \
        -m 4096 \
        -smp 2 \
        -drive if=pflash,format=raw,readonly=on,file="$WORK_DIR/OVMF_CODE.fd" \
        -drive if=pflash,format=raw,file="$WORK_DIR/OVMF_VARS.fd" \
        -drive file="$DISK_IMAGE",format=raw,if=virtio \
        -cdrom "$WORK_DIR/installer.iso" \
        -boot d

    echo "Installation complete!"
    echo "Moving disk image to output directory..."
    FINAL_IMAGE="$OUTPUT_DIR/$(basename "$DISK_IMAGE")"
    mv "$DISK_IMAGE" "$FINAL_IMAGE"
    echo "Generating checksum..."
    sha256sum "$FINAL_IMAGE" > "${FINAL_IMAGE}.sha256"
    echo "Done! Disk image at: $FINAL_IMAGE"

prepare-firmware-arm64:
    #!/usr/bin/env bash
    set -euo pipefail

    WORK_DIR="{{work_dir}}/qemu-arm64"
    mkdir -p "$WORK_DIR"

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

    ln -sf "$AAVMF_CODE" "$WORK_DIR/AAVMF_CODE.fd"
    cp "$AAVMF_VARS_TEMPLATE" "$WORK_DIR/AAVMF_VARS.fd"
    echo "Prepared firmware in $WORK_DIR"

prepare-iso-arm64: create-iso-arm64
    #!/usr/bin/env bash
    set -euo pipefail

    WORK_DIR="{{work_dir}}/qemu-arm64"
    mkdir -p "$WORK_DIR"

    ISO_FILE=$(ls -1t {{autoinstall_dir}}/ubuntu-*-bes-server-arm64-*.iso | head -1)
    if [ -z "$ISO_FILE" ]; then
        echo "ERROR: No ARM64 ISO found in {{autoinstall_dir}}"
        exit 1
    fi

    ln -sf "$(realpath "$ISO_FILE")" "$WORK_DIR/installer.iso"
    echo "Prepared ISO: $ISO_FILE"

qemu-arm64: prepare-firmware-arm64 prepare-iso-arm64
    #!/usr/bin/env bash
    set -euo pipefail

    WORK_DIR="{{work_dir}}/qemu-arm64"
    OUTPUT_DIR="{{output_dir}}/qemu-arm64"
    mkdir -p "$OUTPUT_DIR"

    TIMESTAMP=$(date +%Y%m%d%H%M%S)
    DISK_IMAGE="$WORK_DIR/ubuntu-24.04-arm64-${TIMESTAMP}.raw"

    qemu-img create -f raw "$DISK_IMAGE" 8G

    echo ""
    echo "This will run the automated installer. Wait for it to complete fully,"
    echo "then shut down the machine from the menu."

    qemu-system-aarch64 \
        -machine virt \
        -cpu cortex-a57 \
        -m 4096 \
        -smp 2 \
        -drive if=pflash,format=raw,readonly=on,file="$WORK_DIR/AAVMF_CODE.fd" \
        -drive if=pflash,format=raw,file="$WORK_DIR/AAVMF_VARS.fd" \
        -drive file="$DISK_IMAGE",format=raw,if=virtio \
        -cdrom "$WORK_DIR/installer.iso" \
        -boot d

    echo "Installation complete!"
    echo "Moving disk image to output directory..."
    FINAL_IMAGE="$OUTPUT_DIR/$(basename "$DISK_IMAGE")"
    mv "$DISK_IMAGE" "$FINAL_IMAGE"
    echo "Generating checksum..."
    sha256sum "$FINAL_IMAGE" > "${FINAL_IMAGE}.sha256"
    echo "Done! Disk image at: $FINAL_IMAGE"
