linux_only := if os() == "linux" { "" } else { error("Can only run on Linux") }

ubuntu_version := "24.04.3"
arch := "amd64"

_default:
  @just --list
  @echo "Variable: arch={{arch}} (amd64, arm64)"
  @echo "Variable: ubuntu_version={{ubuntu_version}}"

filestem := "ubuntu-" + ubuntu_version + "-bes-server-" + arch + "-" + datetime_utc("%Y%m%d")

work_dir := "working" / arch
output_dir := "output" / arch

autoinstall := work_dir / "autoinstall.yaml"
output_iso := output_dir / filestem + ".iso"
output_raw := output_dir / filestem + ".raw"
output_vmdk := output_dir / filestem + ".vmdk"
output_qcow := output_dir / filestem + ".qcow2"

qemu_command := if arch == "amd64" { "qemu-system-x86_64" } else if arch == "arm64" { "qemu-system-aarch64" } else { error("Unsupported architecture") }
qemu_options := if arch == "amd64" { if arch() == "x86_64" { "-enable-kvm" } else { "-machine virt" } } else if arch == "arm64" { if arch() == "aarch64" { "-enable-kvm" } else { "-machine virt -cpu cortex-a57" } } else { error("Unsupported architecture") }
qemu_firmware := if arch == "amd64" { work_dir / "OVMF_CODE.fd" } else if arch == "arm64" { work_dir / "AAVMF_CODE.fd" } else { error("Unsupported architecture") }
qemu_firmvars := if arch == "amd64" { work_dir / "OVMF_VARS.fd" } else if arch == "arm64" { work_dir / "AAVMF_VARS.fd" } else { error("Unsupported architecture") }
qemu_memory := "4096"
qemu_cores := "2"

clean:
  mkdir -p "{{work_dir}}" "{{output_dir}}"
  rm -rf "{{work_dir}}"/* "{{output_dir}}"/* || true

generate-autoinstall: clean
  node iso/generate-user-data.js "{{arch}}" > "{{autoinstall}}"

create-iso: generate-autoinstall
  iso/remaster-iso.sh \
    --arch "{{arch}}" \
    --version "{{ubuntu_version}}" \
    --output "{{output_iso}}" \
    --config "{{autoinstall}}"

_prepare-firmware: clean
  #!/usr/bin/env bash
  set -euo pipefail

  if [ "{{arch}}" == "amd64" ]; then
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

    ln -sf "$OVMF_CODE" "{{qemu_firmware}}"
    cp "$OVMF_VARS_TEMPLATE" "{{qemu_firmvars}}"

  elif [ "{{arch}}" == "arm64" ]; then
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

    ln -sf "$AAVMF_CODE" "{{qemu_firmware}}"
    cp "$AAVMF_VARS_TEMPLATE" "{{qemu_firmvars}}"

  else
    echo "Unsupported architecture"
    exit 1
  fi

qemu: _prepare-firmware create-iso clean
  qemu-img create -f raw "{{output_raw}}" 8G
  @echo ""
  @echo "This will run the automated installer. Wait for it to complete fully,"
  @echo "until you see the message '[  OK  ] Finished cloud-final.service',"
  @echo "then power down the machine from the menu."
  "{{qemu_command}}" {{qemu_options}} \
    -m "{{qemu_memory}}" \
    -smp "{{qemu_cores}}" \
    -drive if=pflash,format=raw,readonly=on,file="{{qemu_firmware}}" \
    -drive if=pflash,format=raw,file="{{qemu_firmvars}}" \
    -drive file="{{output_raw}}",format=raw,if=virtio \
    -cdrom "{{output_iso}}" \
    -boot d

vmdk: qemu
  qemu-img convert -f raw -O vmdk -o subformat=streamOptimized "{{output_raw}}" "{{output_vmdk}}"

qcow: qemu
  qemu-img convert -f raw -O qcow2 -o compression_type=zstd "{{output_raw}}" "{{output_qcow}}"

build: qemu vmdk qcow
  #!/usr/bin/env bash
  set -euxo pipefail
  cd "{{output_dir}}"

  for file in *; do
      zstd -T0 -19 "$file"
  done

  sha256sum * | tee SHA256SUMS
