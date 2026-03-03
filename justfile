linux_only := if os() == "linux" { "" } else { error("Can only run on Linux") }

ubuntu_version := "24.04"
ubuntu_suite := "noble"
arch := "amd64"
variant := "metal"
qemu_memory := "4096"
qemu_cores := "2"

_default:
  @echo "{{BOLD}}You probably want to run {{INVERT}}just build{{NORMAL}}"
  @echo ""
  @just --list
  @echo ""
  @echo "Variable: arch={{arch}} (amd64, arm64)"
  @echo "Variable: variant={{variant}} (metal, cloud)"
  @echo "Variable: ubuntu_version={{ubuntu_version}}"
  @echo "Variable: ubuntu_suite={{ubuntu_suite}}"
  @echo "Variable: qemu_memory={{qemu_memory}}"
  @echo "Variable: qemu_cores={{qemu_cores}}"

_validate-variant:
  #!/usr/bin/env bash
  case "{{variant}}" in
    metal|cloud) ;;
    *) echo "ERROR: variant must be one of: metal, cloud (got: {{variant}})"; exit 1 ;;
  esac

_validate-arch:
  #!/usr/bin/env bash
  case "{{arch}}" in
    amd64|arm64) ;;
    *) echo "ERROR: arch must be one of: amd64, arm64 (got: {{arch}})"; exit 1 ;;
  esac

filestem := "ubuntu-" + ubuntu_version + "-bes-" + variant + "-" + arch + "-" + datetime_utc("%Y%m%d")

work_dir := "working" / arch
output_dir := "output" / arch

output_raw := output_dir / filestem + ".raw"
output_vmdk := output_dir / filestem + ".vmdk"
output_qcow := output_dir / filestem + ".qcow2"

# --- QEMU settings for boot tests ---
qemu_command := if arch == "amd64" {
    "qemu-system-x86_64"
  } else if arch == "arm64" {
    "qemu-system-aarch64"
  } else {
    error("Unsupported architecture")
  }

qemu_accel := if arch == "amd64" {
    if arch() == "x86_64" { "-enable-kvm" } else { "-machine virt" }
  } else if arch == "arm64" {
    if arch() == "aarch64" { "-enable-kvm -machine virt" } else { "-machine virt -cpu cortex-a57" }
  } else {
    error("Unsupported architecture")
  }

qemu_firmware := if arch == "amd64" {
    work_dir / "OVMF_CODE.fd"
  } else if arch == "arm64" {
    work_dir / "AAVMF_CODE.fd"
  } else {
    error("Unsupported architecture")
  }

qemu_firmvars := if arch == "amd64" {
    work_dir / "OVMF_VARS.fd"
  } else if arch == "arm64" {
    work_dir / "AAVMF_VARS.fd"
  } else {
    error("Unsupported architecture")
  }

# ============================================================
# Housekeeping
# ============================================================

# Remove all build artifacts
clean:
  mkdir -p "{{work_dir}}" "{{output_dir}}"
  rm -rf "{{work_dir}}"/* "{{output_dir}}"/* || true

# ============================================================
# Image building (Phase 1)
# ============================================================

# Build a raw disk image via debootstrap + chroot
raw: _validate-variant _validate-arch _ensure-dirs
  #!/usr/bin/env bash
  set -euo pipefail
  echo "Building raw image: {{output_raw}}"
  sudo ARCH="{{arch}}" \
       VARIANT="{{variant}}" \
       OUTPUT="{{output_raw}}" \
       IMAGE_SIZE=8G \
       UBUNTU_SUITE="{{ubuntu_suite}}" \
       image/build.sh

# Post-process image (defrag, dedupe, compress)
_post-process-image:
  cd image && docker build -t image-post-process -f Dockerfile.post-process .

_post-process: raw _post-process-image
  docker run --rm --privileged \
    -v "$(pwd)/{{output_dir}}:/work" \
    -v /dev:/dev \
    --init \
    image-post-process post-process "{{filestem}}" "{{variant}}"

# Build raw image and post-process it
image: _post-process

# Convert raw image to VMDK (streamOptimized)
vmdk: image
  qemu-img convert -f raw -O vmdk -o subformat=streamOptimized "{{output_raw}}" "{{output_vmdk}}"

# Convert raw image to qcow2 (zstd compressed)
qcow: image
  qemu-img convert -f raw -O qcow2 -o compression_type=zstd "{{output_raw}}" "{{output_qcow}}"

# Compress raw image with zstd
compress:
  zstd -6 --rm -o '{{output_raw + ".zst"}}' '{{output_raw}}'

# Generate SHA256 checksums for all outputs
checksum:
  cd "{{output_dir}}" && sha256sum * | tee SHA256SUMS

# Build everything: raw + post-process + vmdk + qcow2 + compress + checksum
build: vmdk qcow && compress checksum

# Build all variants for the current architecture
build-all-variants:
  just arch={{arch}} variant=metal build
  just arch={{arch}} variant=cloud build

# Build all variants for all architectures
build-all:
  just arch=amd64 variant=metal build
  just arch=amd64 variant=cloud build
  just arch=arm64 variant=metal build
  just arch=arm64 variant=cloud build

# ============================================================
# Testing
# ============================================================

# Run shellcheck on all shell scripts
test-shellcheck:
  #!/usr/bin/env bash
  set -euo pipefail
  echo "Running shellcheck..."
  find image/ tests/ scripts/ -name '*.sh' -type f -print0 | xargs -0 shellcheck --severity=error
  shellcheck --severity=error image/files/grow-root-filesystem image/files/ts-up image/files/setup-tpm-unlock
  echo "All scripts passed shellcheck."

# Verify image structure by loopback-mounting (requires sudo)
test-structure: _validate-variant _validate-arch
  #!/usr/bin/env bash
  set -euo pipefail
  IMAGE="{{output_raw}}"
  if [ ! -f "$IMAGE" ]; then
    echo "ERROR: image not found: $IMAGE"
    echo "Run 'just raw' first to build the image."
    exit 1
  fi
  sudo tests/test-image-structure.sh "$IMAGE" "{{variant}}" "{{arch}}"

# Prepare QEMU firmware files for boot tests
_prepare-firmware: _ensure-dirs
  #!/usr/bin/env bash
  set -euo pipefail

  if [ "{{arch}}" == "amd64" ]; then
    OVMF_CODE=$(find /usr/share -name 'OVMF_CODE*.fd' -print -quit 2>/dev/null)
    if [ -z "$OVMF_CODE" ]; then
      echo "ERROR: OVMF_CODE.fd not found. Install: apt-get install ovmf"
      exit 1
    fi
    OVMF_VARS=$(find /usr/share -name 'OVMF_VARS*.fd' -print -quit 2>/dev/null)
    if [ -z "$OVMF_VARS" ]; then
      echo "ERROR: OVMF_VARS.fd not found. Install: apt-get install ovmf"
      exit 1
    fi
    ln -sf "$OVMF_CODE" "{{qemu_firmware}}"
    cp "$OVMF_VARS" "{{qemu_firmvars}}"

  elif [ "{{arch}}" == "arm64" ]; then
    AAVMF_CODE=$(find /usr/share -name 'QEMU_CODE.fd' -o -name 'AAVMF_CODE.fd' -o -name 'QEMU_EFI.fd' 2>/dev/null | head -1)
    if [ -z "$AAVMF_CODE" ]; then
      echo "ERROR: AAVMF firmware not found. Install: apt-get install qemu-efi-aarch64"
      exit 1
    fi
    AAVMF_VARS=$(find /usr/share -name 'QEMU_VARS.fd' -o -name 'AAVMF_VARS.fd' 2>/dev/null | head -1)
    if [ -z "$AAVMF_VARS" ]; then
      echo "ERROR: AAVMF_VARS not found. Install: apt-get install qemu-efi-aarch64"
      exit 1
    fi
    ln -sf "$AAVMF_CODE" "{{qemu_firmware}}"
    cp "$AAVMF_VARS" "{{qemu_firmvars}}"
  fi

# Create a cloud-init NoCloud ISO for boot testing
_make-test-cloud-init: _ensure-dirs
  #!/usr/bin/env bash
  set -euo pipefail

  CI_DIR="{{work_dir}}/cidata"
  rm -rf "$CI_DIR"
  mkdir -p "$CI_DIR"

  cat > "$CI_DIR/meta-data" << 'EOF'
  instance-id: test-boot
  local-hostname: test-boot
  EOF

  cat > "$CI_DIR/user-data" << 'CLOUDINIT'
  #cloud-config
  runcmd:
    - |
      #!/bin/bash
      exec > /dev/ttyS0 2>&1

      PASS=0
      FAIL=0
      ERRORS=()

      check() {
        local desc="$1"; shift
        if "$@" >/dev/null 2>&1; then
          echo "PASS: $desc"
          ((PASS++))
        else
          echo "FAIL: $desc"
          ERRORS+=("$desc")
          ((FAIL++))
        fi
      }

      echo "=== BES Boot Smoke Test ==="
      echo ""

      # r[verify test.boot.checks]
      check "systemd reached multi-user.target" systemctl is-active multi-user.target
      FAILED_UNITS=$(systemctl --failed --no-legend --no-pager | wc -l)
      check "no failed systemd units" test "$FAILED_UNITS" -eq 0

      check "sshd is active" systemctl is-active ssh
      check "ufw is active" systemctl is-active ufw
      check "tailscaled is active" systemctl is-active tailscaled
      check "snapper-timeline.timer is active" systemctl is-active snapper-timeline.timer
      check "grow-root-filesystem ran" systemctl show -p ActiveState grow-root-filesystem.service | grep -q inactive

      check "root is btrfs" stat -f -c%T /
      check "compression active in /proc/mounts" grep -q 'compress=' /proc/mounts

      VARIANT=$(cat /etc/bes/image-variant 2>/dev/null || echo "unknown")
      echo "Variant: $VARIANT"

      if [ "$VARIANT" = "metal" ]; then
        check "LUKS volume is active" test -e /dev/mapper/root
      fi

      check "ubuntu user exists" id ubuntu
      check "machine-id is non-empty" test -s /etc/machine-id

      check "/boot is mounted" mountpoint -q /boot
      check "/boot/efi is mounted" mountpoint -q /boot/efi

      echo ""
      echo "RESULTS: $PASS passed, $FAIL failed"

      if [ $FAIL -eq 0 ]; then
        # r[verify test.boot.output]
        echo "TEST_SUCCESS"
      else
        echo "TEST_FAILURE"
        for e in "${ERRORS[@]}"; do
          echo "  - $e"
        done
      fi

      # r[verify test.boot.poweroff]
      sleep 2
      poweroff
  CLOUDINIT

  # Build the NoCloud ISO
  genisoimage -output "{{work_dir}}/cidata.iso" \
    -volid cidata -joliet -rock \
    "$CI_DIR/meta-data" "$CI_DIR/user-data"

# Boot the image in QEMU and run cloud-init smoke tests
test-boot: _validate-variant _validate-arch _prepare-firmware _make-test-cloud-init
  #!/usr/bin/env bash
  set -euo pipefail

  IMAGE="{{output_raw}}"
  if [ ! -f "$IMAGE" ]; then
    echo "ERROR: image not found: $IMAGE"
    echo "Run 'just raw' first."
    exit 1
  fi

  # Make a copy so we don't modify the original
  TEST_IMAGE="{{work_dir}}/test-boot.raw"
  cp "$IMAGE" "$TEST_IMAGE"

  # Grow the test image so grow-root-filesystem has something to do
  qemu-img resize "$TEST_IMAGE" 12G

  SERIAL_LOG="{{work_dir}}/test-boot-serial.log"
  TIMEOUT={{qemu_memory}}  # reuse as a rough proxy — actually use 300s
  TIMEOUT=300

  echo "Booting image in QEMU (timeout: ${TIMEOUT}s)..."
  echo "Serial log: $SERIAL_LOG"

  # r[verify test.boot.method]
  timeout "$TIMEOUT" \
    {{qemu_command}} {{qemu_accel}} \
    -m {{qemu_memory}} \
    -smp {{qemu_cores}} \
    -nographic \
    -serial mon:stdio \
    -drive if=pflash,format=raw,readonly=on,file="{{qemu_firmware}}" \
    -drive if=pflash,format=raw,file="{{qemu_firmvars}}" \
    -drive file="$TEST_IMAGE",format=raw,if=virtio \
    -drive file="{{work_dir}}/cidata.iso",format=raw,if=virtio \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0 \
    -no-reboot \
    2>&1 | tee "$SERIAL_LOG" || true

  echo ""
  echo "=== Checking test results ==="

  # r[verify test.boot.timeout]
  if grep -q "TEST_SUCCESS" "$SERIAL_LOG"; then
    echo "Boot smoke test PASSED"
    exit 0
  elif grep -q "TEST_FAILURE" "$SERIAL_LOG"; then
    echo "Boot smoke test FAILED"
    grep "FAIL:" "$SERIAL_LOG" || true
    exit 1
  else
    echo "Boot smoke test TIMED OUT or did not complete"
    echo "Last 30 lines of serial log:"
    tail -30 "$SERIAL_LOG"
    exit 1
  fi

# Run all tests (structure + boot if KVM available)
test: test-shellcheck test-structure
  #!/usr/bin/env bash
  set -euo pipefail
  if [ -e /dev/kvm ]; then
    echo "KVM available — running boot test..."
    just arch={{arch}} variant={{variant}} test-boot
  else
    echo "KVM not available — skipping boot test"
  fi

# ============================================================
# Helpers
# ============================================================

_ensure-dirs:
  @mkdir -p "{{work_dir}}" "{{output_dir}}"
