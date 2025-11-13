iso_dir := "iso"
work_dir := "working"
output_dir := "output"

# Show available recipes
default:
  @just --list

# Generate autoinstall user-data from scripts
generate-autoinstall arch="amd64":
  cd {{iso_dir}} && node generate-user-data.js "user-data-{{arch}}" "{{arch}}"

create-iso arch="amd64": (generate-autoinstall arch)
  cd {{iso_dir}} && ./remaster-iso.sh --arch "{{arch}}" --user-data user-data-"{{arch}}"

prepare-iso arch="amd64": (create-iso arch)
  #!/usr/bin/env bash
  set -euo pipefail

  WORK_DIR="{{work_dir}}/{{arch}}"
  mkdir -p "$WORK_DIR"

  ISO_FILE=$(ls -1t "{{iso_dir}}/ubuntu"-*-"bes-server-{{arch}}"-*.iso | head -1)
  ln -sf "$(realpath "$ISO_FILE")" "$WORK_DIR/installer.iso"

prepare-firmware-amd64:
  #!/usr/bin/env bash
  set -euo pipefail

  WORK_DIR="{{work_dir}}/amd64"
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

prepare-firmware-arm64:
  #!/usr/bin/env bash
  set -euo pipefail

  WORK_DIR="{{work_dir}}/arm64"
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

prepare-firmware arch="amd64":
  just $(echo "prepare-firmware-{{arch}}")

qemu-amd64: (prepare-firmware "amd64") (prepare-iso "amd64")
  #!/usr/bin/env bash
  set -euo pipefail

  WORK_DIR="{{work_dir}}/amd64"
  OUTPUT_DIR="{{output_dir}}/amd64"
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
  echo "Generating checksum for raw image..."
  sha256sum "$FINAL_IMAGE" > "${FINAL_IMAGE}.sha256"
  echo "Compressing with zstd..."
  zstd -9 "$FINAL_IMAGE" -o "${FINAL_IMAGE}.zst"
  echo "Generating checksum for compressed image..."
  sha256sum "${FINAL_IMAGE}.zst" > "${FINAL_IMAGE}.zst.sha256"
  echo "Done! Raw image: $FINAL_IMAGE"
  echo "      Compressed: ${FINAL_IMAGE}.zst"

qemu-arm64: (prepare-firmware "arm64") (prepare-iso "arm64")
  #!/usr/bin/env bash
  set -euo pipefail

  WORK_DIR="{{work_dir}}/arm64"
  OUTPUT_DIR="{{output_dir}}/arm64"
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
  FINAL_IMAGE="$OUTPUT_DIR/$(basename "$DISK_IMAGE")"
  mv "$DISK_IMAGE" "$FINAL_IMAGE"

qemu arch="amd64":
  just $(echo qemu-{{arch}})

convert-vmdk arch="amd64": (qemu arch)
  #!/usr/bin/env bash
  set -euo pipefail

  OUTPUT_DIR="{{output_dir}}/{{arch}}"
  RAW_IMAGE=$(ls -1t "$OUTPUT_DIR"/*.raw | head -1)
  BASENAME="${RAW_IMAGE%.raw}"

  qemu-img convert -f raw -O vmdk -o subformat=streamOptimized "$RAW_IMAGE" "${BASENAME}.vmdk"

convert-qcow arch="amd64": (qemu arch)
  #!/usr/bin/env bash
  set -euo pipefail

  OUTPUT_DIR="{{output_dir}}/{{arch}}"
  RAW_IMAGE=$(ls -1t "$OUTPUT_DIR"/*.raw | head -1)
  BASENAME="${RAW_IMAGE%.raw}"

  qemu-img convert -f raw -O qcow2 -o compression_type=zstd "$RAW_IMAGE" "${BASENAME}.qcow2"

convert-ova arch="amd64": (convert-vmdk arch)
  #!/usr/bin/env bash
  set -euo pipefail

  OUTPUT_DIR="{{output_dir}}/{{arch}}"
  RAW_IMAGE=$(ls -1t "$OUTPUT_DIR"/*.raw | head -1)
  BASENAME="${RAW_IMAGE%.raw}"
  VMDK_FILE="${BASENAME}.vmdk"
  OVF_FILE="${BASENAME}.ovf"
  OVA_FILE="${BASENAME}.ova"

  VMDK_SIZE=$(stat -c%s "$VMDK_FILE")
  DISK_SIZE=$(qemu-img info "$RAW_IMAGE" | grep 'virtual size' | awk '{print $3}')

  sed -e "s|__VMDK_FILENAME__|$(basename "$VMDK_FILE")|g" \
      -e "s|__VMDK_SIZE__|$VMDK_SIZE|g" \
      -e "s|__DISK_SIZE__|$DISK_SIZE|g" \
      templates/ubuntu-{{arch}}.ovf.template > "$OVF_FILE"

  tar -cf "$OVA_FILE" -C "$(dirname "$OVF_FILE")" "$(basename "$OVF_FILE")" "$(basename "$VMDK_FILE")"
  rm "$OVF_FILE"

build arch="amd64": (qemu arch) (convert-vmdk arch) (convert-qcow arch) (convert-ova arch)
  #!/usr/bin/env bash
  set -euo pipefail
  cd "{{output_dir}}/{{arch}}"

  for file in *; do
      zstd -T0 -19 "$file"
  done

  sha256sum * | tee SHA256SUMS
