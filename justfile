linux_only := if os() == "linux" { "" } else { error("Can only run on Linux") }

ubuntu_version := "24.04.3"
arch := "amd64"
variant := "metal-encrypted"
qemu_memory := "4096"
qemu_cores := "2"

_default:
  @echo "{{BOLD}}You probably want to run {{INVERT}}just build{{NORMAL}}"
  @echo ""
  @just --list
  @echo ""
  @echo "Variable: arch={{arch}} (amd64, arm64)"
  @echo "Variable: variant={{variant}} (cloud, metal, metal-encrypted)"
  @echo "Variable: ubuntu_version={{ubuntu_version}}"
  @echo "Variable: qemu_memory={{qemu_memory}}"
  @echo "Variable: qemu_cores={{qemu_cores}}"

_validate-variant:
  #!/usr/bin/env bash
  case "{{variant}}" in
    cloud|metal|metal-encrypted) ;;
    *) echo "ERROR: variant must be one of: cloud, metal, metal-encrypted"; exit 1 ;;
  esac

filestem := "ubuntu-" + ubuntu_version + "-bes-" + variant + "-" + arch + "-" + datetime_utc("%Y%m%d")

work_dir := "working" / arch
output_dir := "output" / arch

autoinstall := work_dir / "autoinstall.yaml"
output_iso := output_dir / filestem + ".iso"
output_raw := output_dir / filestem + ".raw"
output_vmdk := output_dir / filestem + ".vmdk"
output_qcow := output_dir / filestem + ".qcow2"

qemu_command := (if arch == "amd64" {
    "qemu-system-x86_64"
  } else if arch == "arm64" {
    "qemu-system-aarch64"
  } else {
    error("Unsupported architecture")
  })
qemu_options := (if arch == "amd64" {
    if arch() == "x86_64" { "-enable-kvm" } else { "-machine virt" }
  } else if arch == "arm64" {
    if arch() == "aarch64" { "-enable-kvm" } else { "-machine virt -cpu cortex-a57" }
  } else {
    error("Unsupported architecture")
  })
qemu_firmware := (if arch == "amd64" {
    work_dir / "OVMF_CODE.fd"
  } else if arch == "arm64" {
    work_dir / "AAVMF_CODE.fd"
  } else {
    error("Unsupported architecture")
  })
qemu_firmvars := (if arch == "amd64" {
    work_dir / "OVMF_VARS.fd"
  } else if arch == "arm64" {
    work_dir / "AAVMF_VARS.fd"
  } else {
    error("Unsupported architecture")
  })

clean:
  mkdir -p "{{work_dir}}" "{{output_dir}}"
  rm -rf "{{work_dir}}"/* "{{output_dir}}"/* || true

_generate-autoinstall: clean _validate-variant
  node iso/generate-user-data.js "{{arch}}" "{{variant}}" > "{{autoinstall}}"

iso: _generate-autoinstall
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
    OVMF_CODE=$(find /usr/share -name 'OVMF_CODE*.fd' -print -quit 2>/dev/null)
    if [ -z "$OVMF_CODE" ]; then
      echo "ERROR: OVMF_CODE.fd not found. Install ovmf package."
      echo "Searched under /usr/share:"
      find /usr/share -iname '*ovmf*' 2>/dev/null || true
      exit 1
    fi
    echo "Using OVMF_CODE: $OVMF_CODE"

    OVMF_VARS_TEMPLATE=$(find /usr/share -name 'OVMF_VARS*.fd' -print -quit 2>/dev/null)
    if [ -z "$OVMF_VARS_TEMPLATE" ]; then
      echo "ERROR: OVMF_VARS.fd not found. Install ovmf package."
      echo "Searched under /usr/share:"
      find /usr/share -iname '*ovmf*' 2>/dev/null || true
      exit 1
    fi
    echo "Using OVMF_VARS: $OVMF_VARS_TEMPLATE"

    ln -sf "$OVMF_CODE" "{{qemu_firmware}}"
    cp "$OVMF_VARS_TEMPLATE" "{{qemu_firmvars}}"

  elif [ "{{arch}}" == "arm64" ]; then
    # Find AAVMF firmware files for ARM64
    AAVMF_CODE=$(find /usr/share -name 'QEMU_CODE.fd' -o -name 'AAVMF_CODE.fd' -o -name 'QEMU_EFI.fd' -o -name 'QEMU_EFI-pflash.raw' 2>/dev/null | head -1)
    if [ -z "$AAVMF_CODE" ]; then
      echo "ERROR: AAVMF/QEMU_EFI firmware not found. Install qemu-efi-aarch64 package."
      echo "Searched under /usr/share:"
      find /usr/share -iname '*aavmf*' -o -iname '*qemu_efi*' 2>/dev/null || true
      exit 1
    fi
    echo "Using AAVMF_CODE: $AAVMF_CODE"

    AAVMF_VARS_TEMPLATE=$(find /usr/share -name 'QEMU_VARS.fd' -o -name 'AAVMF_VARS.fd' -o -name 'vars-template-pflash.raw' 2>/dev/null | head -1)
    if [ -z "$AAVMF_VARS_TEMPLATE" ]; then
      echo "ERROR: AAVMF_VARS firmware not found. Install qemu-efi-aarch64 package."
      echo "Searched under /usr/share:"
      find /usr/share -iname '*aavmf*' -o -iname '*qemu_vars*' -o -iname '*pflash*' 2>/dev/null || true
      exit 1
    fi
    echo "Using AAVMF_VARS: $AAVMF_VARS_TEMPLATE"

    ln -sf "$AAVMF_CODE" "{{qemu_firmware}}"
    cp "$AAVMF_VARS_TEMPLATE" "{{qemu_firmvars}}"

  else
    echo "Unsupported architecture"
    exit 1
  fi

_qemu: _prepare-firmware iso
  qemu-img create -f raw "{{output_raw}}" 8G
  {{qemu_command}} {{qemu_options}} \
    -m {{qemu_memory}} \
    -smp {{qemu_cores}} \
    -no-reboot \
    -drive if=pflash,format=raw,readonly=on,file="{{qemu_firmware}}" \
    -drive if=pflash,format=raw,file="{{qemu_firmvars}}" \
    -drive file="{{output_raw}}",format=raw,if=virtio \
    -cdrom "{{output_iso}}" \
    -boot d

_post-process-image:
  cd iso && docker build -t image-post-process -f Dockerfile.post-process .

_post-process: _qemu _post-process-image
  docker run --rm --privileged -v "$(pwd)/{{output_dir}}:/work" -v /dev:/dev --init image-post-process post-process "{{filestem}}" "{{variant}}"

raw: _post-process

vmdk: _post-process
  qemu-img convert -f raw -O vmdk -o subformat=streamOptimized "{{output_raw}}" "{{output_vmdk}}"

qcow: _post-process
  qemu-img convert -f raw -O qcow2 -o compression_type=zstd "{{output_raw}}" "{{output_qcow}}"

build: raw vmdk qcow && compress checksum

compress:
  zstd -6 --rm -o '{{output_raw + ".zst"}}' '{{output_raw}}'

checksum:
  cd "{{output_dir}}" && sha256sum * | tee SHA256SUMS
