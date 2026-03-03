linux_only := if os() == "linux" { "" } else { error("Can only run on Linux") }

# Auto-detect container runtime: prefer docker, fall back to podman
container_runtime := if `command -v docker >/dev/null 2>&1 && echo found || echo missing` == "found" {
    "docker"
  } else if `command -v podman >/dev/null 2>&1 && echo found || echo missing` == "found" {
    "sudo podman"
  } else {
    error("Neither docker nor podman found")
  }

ubuntu_version := "24.04"
ubuntu_suite := "noble"
arch := "amd64"
variant := "metal"
qemu_memory := "4096"
qemu_cores := "2"

# Mirror for debootstrap: override via env var or `just ubuntu_mirror=...`
ubuntu_mirror := if arch == "arm64" {
    env_var_or_default("UBUNTU_PORTS_MIRROR", "http://ports.ubuntu.com/ubuntu-ports")
  } else {
    env_var_or_default("UBUNTU_MIRROR", "http://nz.archive.ubuntu.com/ubuntu")
  }

_default:
  @echo "{{BOLD}}You probably want to run {{INVERT}}just build{{NORMAL}}"
  @echo ""
  @just --list
  @echo ""
  @echo "Variable: arch={{arch}} (amd64, arm64)"
  @echo "Variable: variant={{variant}} (metal, cloud)"
  @echo "Variable: ubuntu_version={{ubuntu_version}}"
  @echo "Variable: ubuntu_suite={{ubuntu_suite}}"
  @echo "Variable: ubuntu_mirror={{ubuntu_mirror}}"
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
output_iso := output_dir / "bes-installer-" + arch + ".iso"

# --- Rust installer settings ---
cargo_target := if arch == "amd64" {
    "x86_64-unknown-linux-musl"
  } else if arch == "arm64" {
    "aarch64-unknown-linux-musl"
  } else {
    error("Unsupported architecture")
  }

installer_bin := "installer/tui/target" / cargo_target / "release" / "bes-installer"

# --- QEMU settings for boot tests ---
qemu_command := if arch == "amd64" {
    "qemu-system-x86_64"
  } else if arch == "arm64" {
    "qemu-system-aarch64"
  } else {
    error("Unsupported architecture")
  }

qemu_accel := if arch == "amd64" {
    if arch() == "x86_64" { "-accel kvm -accel tcg" } else { "-accel tcg" }
  } else if arch == "arm64" {
    if arch() == "aarch64" { "-accel kvm -accel tcg -machine virt" } else { "-accel tcg -machine virt -cpu cortex-a57" }
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
# Installer (Rust TUI)
# ============================================================

# Build the TUI installer binary (static musl)
installer-build: _validate-arch
  cd installer/tui && cargo build --release --target {{cargo_target}}

# Run installer unit tests
installer-test:
  cd installer/tui && cargo test

# Run clippy and fmt checks on the installer
installer-lint:
  cd installer/tui && cargo fmt --check && cargo clippy -- -D warnings

# ============================================================
# Live ISO
# ============================================================

# Build the live installer ISO (requires images + installer binary)
iso: _validate-arch installer-build
  #!/usr/bin/env bash
  set -euo pipefail

  # Verify we have images for both variants
  METAL_IMAGE="$(find "{{output_dir}}" -maxdepth 1 -name '*-metal-*.raw.zst' | head -1)"
  CLOUD_IMAGE="$(find "{{output_dir}}" -maxdepth 1 -name '*-cloud-*.raw.zst' | head -1)"

  if [ -z "$METAL_IMAGE" ] || [ -z "$CLOUD_IMAGE" ]; then
    echo "ERROR: need both metal and cloud .raw.zst images in {{output_dir}}"
    echo "Run 'just arch={{arch}} variant=metal build' and 'just arch={{arch}} variant=cloud build' first."
    exit 1
  fi

  sudo ARCH="{{arch}}" \
       OUTPUT="{{output_iso}}" \
       INSTALLER_BIN="{{installer_bin}}" \
       IMAGE_DIR="{{output_dir}}" \
       UBUNTU_SUITE="{{ubuntu_suite}}" \
       UBUNTU_MIRROR="{{ubuntu_mirror}}" \
       iso/build-iso.sh

# ============================================================
# Housekeeping
# ============================================================

# Check for all required and optional dependencies
check-deps:
  #!/usr/bin/env bash
  PASS=0
  FAIL=0
  OPTIONAL_FAIL=0

  req() {
    if command -v "$1" >/dev/null 2>&1; then
      echo "  ✓ $1 $(command -v "$1")"
      ((PASS++))
    else
      echo "  ✗ $1 — $2"
      ((FAIL++))
    fi
  }

  opt() {
    if command -v "$1" >/dev/null 2>&1; then
      echo "  ✓ $1 $(command -v "$1")"
      ((PASS++))
    else
      echo "  ○ $1 — $2"
      ((OPTIONAL_FAIL++))
    fi
  }

  echo "=== Required: image build (just raw) ==="
  req debootstrap    "Arch: pacman -S debootstrap / Debian: apt install debootstrap"
  req sgdisk         "Arch: pacman -S gptfdisk / Debian: apt install gdisk"
  req mkfs.vfat      "Arch: pacman -S dosfstools / Debian: apt install dosfstools"
  req mkfs.ext4      "Arch: pacman -S e2fsprogs / Debian: apt install e2fsprogs"
  req mkfs.btrfs     "Arch: pacman -S btrfs-progs / Debian: apt install btrfs-progs"
  req btrfs          "Arch: pacman -S btrfs-progs / Debian: apt install btrfs-progs"
  req losetup        "Arch: pacman -S util-linux / Debian: apt install util-linux"
  req cryptsetup     "Arch: pacman -S cryptsetup / Debian: apt install cryptsetup"
  req partprobe      "Arch: pacman -S parted / Debian: apt install parted"
  req udevadm        "Arch: pacman -S systemd / Debian: apt install udev"
  req truncate       "Arch: pacman -S coreutils / Debian: apt install coreutils"
  req chroot         "Arch: pacman -S coreutils / Debian: apt install coreutils"
  req rsync          "Arch: pacman -S rsync / Debian: apt install rsync"
  echo ""

  echo "=== Required: post-processing & output (just build) ==="
  if command -v docker >/dev/null 2>&1; then
    echo "  ✓ docker $(command -v docker)"
    ((PASS++))
  elif command -v podman >/dev/null 2>&1; then
    echo "  ✓ podman $(command -v podman) (used as container runtime)"
    ((PASS++))
  else
    echo "  ✗ docker or podman — need at least one container runtime"
    ((FAIL++))
  fi
  req qemu-img       "Arch: pacman -S qemu-img / Debian: apt install qemu-utils"
  req zstd           "Arch: pacman -S zstd / Debian: apt install zstd"
  req sha256sum      "Arch: pacman -S coreutils / Debian: apt install coreutils"
  echo ""

  echo "=== Required: testing (just test) ==="
  req shellcheck     "Arch: pacman -S shellcheck / Debian: apt install shellcheck"
  req blkid          "Arch: pacman -S util-linux / Debian: apt install util-linux"
  echo ""

  echo "=== Optional: boot smoke tests (just test-boot) ==="
  opt qemu-system-x86_64  "Arch: pacman -S qemu-system-x86 / Debian: apt install qemu-system-x86"
  opt qemu-system-aarch64 "Arch: pacman -S qemu-system-aarch64 / Debian: apt install qemu-system-arm"
  opt genisoimage         "Arch: pacman -S cdrtools / Debian: apt install genisoimage"
  if [ -e /dev/kvm ]; then
    echo "  ✓ /dev/kvm is available"
    ((PASS++))
  else
    echo "  ○ /dev/kvm not available — boot tests will be slow or skipped"
    ((OPTIONAL_FAIL++))
  fi

  FIRMWARE_FOUND=0
  for f in /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2/x64/OVMF_CODE.fd /usr/share/edk2-ovmf/x64/OVMF_CODE.4m.fd; do
    if [ -f "$f" ]; then FIRMWARE_FOUND=1; break; fi
  done
  if [ $FIRMWARE_FOUND -eq 1 ]; then
    echo "  ✓ UEFI firmware found ($f)"
    ((PASS++))
  else
    echo "  ○ UEFI firmware not found — Arch: pacman -S edk2-ovmf / Debian: apt install ovmf"
    ((OPTIONAL_FAIL++))
  fi
  echo ""

  echo "=== Optional: ISO build (just iso) ==="
  opt mksquashfs     "Arch: pacman -S squashfs-tools / Debian: apt install squashfs-tools"
  opt grub-mkimage   "Arch: pacman -S grub / Debian: apt install grub-common"
  opt xorriso        "Arch: pacman -S xorriso / Debian: apt install xorriso"
  echo ""

  echo "=== Optional: installer build (just installer-build) ==="
  if command -v rustup >/dev/null 2>&1; then
    echo "  ✓ rustup $(command -v rustup)"
    ((PASS++))
    if rustup target list --installed 2>/dev/null | grep -q "$(uname -m | sed 's/x86_64/x86_64-unknown-linux-musl/;s/aarch64/aarch64-unknown-linux-musl/')"; then
      echo "  ✓ musl target installed"
      ((PASS++))
    else
      echo "  ○ musl target not installed — run: rustup target add x86_64-unknown-linux-musl"
      ((OPTIONAL_FAIL++))
    fi
  else
    echo "  ○ rustup — install from https://rustup.rs"
    ((OPTIONAL_FAIL++))
  fi
  if [ -f /usr/lib/musl/lib/libc.a ] || [ -f /lib/ld-musl-x86_64.so.1 ] || [ -f /lib/ld-musl-aarch64.so.1 ]; then
    echo "  ✓ musl libc found"
    ((PASS++))
  else
    echo "  ○ musl libc — Arch: pacman -S musl / Debian: apt install musl-tools"
    ((OPTIONAL_FAIL++))
  fi
  echo ""

  echo "=== Optional: cross-architecture builds ==="
  if [ -f /proc/sys/fs/binfmt_misc/qemu-aarch64 ]; then
    echo "  ✓ binfmt qemu-aarch64 registered"
    ((PASS++))
  else
    echo "  ○ binfmt qemu-aarch64 not registered — Arch: pacman -S qemu-user-static-binfmt / Debian: apt install qemu-user-static binfmt-support"
    ((OPTIONAL_FAIL++))
  fi
  echo ""

  echo "=============================="
  echo "$PASS found, $FAIL missing, $OPTIONAL_FAIL optional missing"
  echo "=============================="
  if [ $FAIL -gt 0 ]; then
    echo "Install the missing required tools above before building."
    exit 1
  fi
  if [ $OPTIONAL_FAIL -gt 0 ]; then
    echo "Optional tools are only needed for specific tasks — see labels above."
  fi

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
  if [ -f "{{output_raw}}" ]; then
    echo "Raw image already exists: {{output_raw}} (skipping build)"
    exit 0
  fi
  echo "Building raw image: {{output_raw}}"
  sudo ARCH="{{arch}}" \
       VARIANT="{{variant}}" \
       OUTPUT="{{output_raw}}" \
       IMAGE_SIZE=8G \
       UBUNTU_SUITE="{{ubuntu_suite}}" \
       UBUNTU_MIRROR="{{ubuntu_mirror}}" \
       image/build.sh

# Post-process image (defrag, dedupe, compress)
_post-process-image:
  cd image && {{container_runtime}} build -t image-post-process -f Dockerfile.post-process .

_post-process: raw _post-process-image
  {{container_runtime}} run --rm --privileged \
    -v "$(pwd)/{{output_dir}}:/work" \
    -v /dev:/dev \
    --init \
    image-post-process post-process "{{filestem}}" "{{variant}}"

# Build raw image and post-process it
# r[image.output.raw]
image: _post-process

# Convert raw image to VMDK (streamOptimized)
# r[image.output.vmdk]
vmdk: image
  qemu-img convert -f raw -O vmdk -o subformat=streamOptimized "{{output_raw}}" "{{output_vmdk}}"

# Convert raw image to qcow2 (zstd compressed)
# r[image.output.qcow2]
qcow: image
  qemu-img convert -f raw -O qcow2 -o compression_type=zstd "{{output_raw}}" "{{output_qcow}}"

# Compress raw image with zstd
# r[image.output.compress]
compress:
  zstd -6 --rm -o '{{output_raw + ".zst"}}' '{{output_raw}}'

# Generate SHA256 checksums for all outputs
# r[image.output.checksum]
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
  find image/ tests/ scripts/ iso/ -name '*.sh' -type f -print0 | xargs -0 shellcheck --severity=error
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

# Verify ISO structure without booting (requires sudo)
iso-test-structure: _validate-arch
  #!/usr/bin/env bash
  set -euo pipefail
  ISO="{{output_iso}}"
  if [ ! -f "$ISO" ]; then
    echo "ERROR: ISO not found: $ISO"
    echo "Run 'just iso' first to build the ISO."
    exit 1
  fi
  sudo tests/test-iso-structure.sh "$ISO" "{{arch}}"

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

# Run E2E install test: boot ISO in QEMU, auto-install to blank disk, verify
test-e2e: _validate-variant _validate-arch
  #!/usr/bin/env bash
  set -euo pipefail
  ISO="{{output_iso}}"
  if [ ! -f "$ISO" ]; then
    echo "ERROR: ISO not found: $ISO"
    echo "Run 'just iso' first to build the live installer."
    exit 1
  fi
  if [ ! -e /dev/kvm ]; then
    echo "ERROR: KVM required for E2E tests"
    exit 1
  fi
  sudo tests/test-e2e-install.sh "$ISO" "{{variant}}" "{{arch}}"

# Run all tests (structure + installer + boot if KVM available)
test: test-shellcheck installer-test test-structure
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
