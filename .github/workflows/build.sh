# r[impl ci.uptodate] Keep all `uses:` actions up to date (see dependabot.yml)
name: Build Images

on:
  pull_request:
  push:
    branches: [main]
    tags: ["v*"]

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: write

env:
  UBUNTU_SUITE: noble # r[impl ci.output-suite] r[verify ci.output-suite]
  UBUNTU_MIRROR: http://us.archive.ubuntu.com/ubuntu
  UBUNTU_PORTS_MIRROR: http://ports.ubuntu.com/ubuntu-ports

jobs:
  images-cloud:
    strategy:
      fail-fast: false
      matrix:
        arch: [amd64, arm64] # r[impl ci.output-arch] r[verify ci.output-arch]
        include:
          - arch: amd64
            runner: ubuntu-24.04
          - arch: arm64
            runner: ubuntu-24.04-arm
    runs-on: ${{ matrix.runner }}

    steps:
      - uses: actions/checkout@v6
      - uses: extractions/setup-just@v3

      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            debootstrap gdisk dosfstools e2fsprogs btrfs-progs \
            cryptsetup parted util-linux rsync shellcheck \
            qemu-utils genisoimage zstd squashfs-tools jq

      - name: Run shellcheck # r[impl ci.shellcheck] r[verify ci.shellcheck]
        run: just test-shellcheck

      - name: Build raw image
        run: just arch=${{ matrix.arch }} variant=cloud raw
        timeout-minutes: 60

      - name: Test image structure
        run: just arch=${{ matrix.arch }} variant=cloud test-structure

      - name: Produce final artifacts
        run: just arch=${{ matrix.arch }} variant=cloud build
        timeout-minutes: 30

      - name: Verify outputs # r[verify image.output.raw] r[verify image.output.vmdk] r[verify image.output.qcow2] r[verify image.output.checksum]
        run: just arch=${{ matrix.arch }} variant=cloud verify-outputs

      - name: List outputs
        run: ls -lh output/${{ matrix.arch }}/cloud/

      - name: Upload raw image (needed by ISO build)
        uses: actions/upload-artifact@v7
        with:
          name: image-raw-cloud-${{ matrix.arch }}
          path: |
            output/${{ matrix.arch }}/cloud/*.raw.zst
            output/${{ matrix.arch }}/cloud/*.raw.size
          if-no-files-found: error
          retention-days: 1
          compression-level: 0

      - name: Upload converted formats (needed by release)
        uses: actions/upload-artifact@v7
        with:
          name: image-formats-cloud-${{ matrix.arch }}
          path: |
            output/${{ matrix.arch }}/cloud/*.vmdk
            output/${{ matrix.arch }}/cloud/*.qcow2
          if-no-files-found: error
          retention-days: 1
          compression-level: 0

  images-metal:
    strategy:
      fail-fast: false
      matrix:
        arch: [amd64, arm64]
        include:
          - arch: amd64
            runner: ubuntu-24.04
          - arch: arm64
            runner: ubuntu-24.04-arm
    runs-on: ${{ matrix.runner }}

    steps:
      - uses: actions/checkout@v6
      - uses: extractions/setup-just@v3

      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            debootstrap gdisk dosfstools e2fsprogs btrfs-progs \
            cryptsetup parted util-linux rsync \
            qemu-utils genisoimage zstd squashfs-tools jq

      - name: Build raw image
        run: just arch=${{ matrix.arch }} variant=metal raw
        timeout-minutes: 60

      - name: Test image structure
        run: just arch=${{ matrix.arch }} variant=metal test-structure

      - name: Produce final artifacts
        run: just arch=${{ matrix.arch }} variant=metal build
        timeout-minutes: 30

      - name: Verify outputs # r[verify image.output.raw] r[verify image.output.vmdk] r[verify image.output.qcow2] r[verify image.output.checksum]
        run: just arch=${{ matrix.arch }} variant=metal verify-outputs

      - name: List outputs
        run: ls -lh output/${{ matrix.arch }}/metal/

      - name: Upload raw image (needed by release)
        uses: actions/upload-artifact@v7
        with:
          name: image-raw-metal-${{ matrix.arch }}
          path: |
            output/${{ matrix.arch }}/metal/*.raw.zst
            output/${{ matrix.arch }}/metal/*.raw.size
          if-no-files-found: error
          retention-days: 1
          compression-level: 0

      - name: Upload converted formats (needed by release)
        uses: actions/upload-artifact@v7
        with:
          name: image-formats-metal-${{ matrix.arch }}
          path: |
            output/${{ matrix.arch }}/metal/*.vmdk
            output/${{ matrix.arch }}/metal/*.qcow2
          if-no-files-found: error
          retention-days: 1
          compression-level: 0

  installer:
    strategy:
      fail-fast: false
      matrix:
        arch: [amd64, arm64]
        include:
          - arch: amd64
            runner: ubuntu-24.04
            cargo_target: x86_64-unknown-linux-musl
          - arch: arm64
            runner: ubuntu-24.04-arm
            cargo_target: aarch64-unknown-linux-musl
    runs-on: ${{ matrix.runner }}
    steps:
      - uses: actions/checkout@v6

      # r[impl ci.rust-stable] r[verify ci.rust-stable]
      - name: Install Rust toolchain via rustup
        run: |
          rustup update stable
          rustup default stable
          rustup target add ${{ matrix.cargo_target }}

      - name: Install musl tools
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends musl-tools

      # r[impl ci.rust-cache] r[verify ci.rust-cache]
      - uses: Swatinem/rust-cache@v2

      - name: Build release binary (static musl)
        run: cargo build --release --target ${{ matrix.cargo_target }} -p bes-installer

      - name: Verify binary is static
        run: |
          file target/${{ matrix.cargo_target }}/release/bes-installer
          ldd target/${{ matrix.cargo_target }}/release/bes-installer 2>&1 | grep -q "not a dynamic" || \
            ldd target/${{ matrix.cargo_target }}/release/bes-installer 2>&1 | grep -q "statically linked" || \
            echo "::warning::Binary may not be fully static"

      - name: Upload installer binary
        uses: actions/upload-artifact@v7
        with:
          name: installer-${{ matrix.arch }}
          path: target/${{ matrix.cargo_target }}/release/bes-installer
          if-no-files-found: error
          retention-days: 1
          compression-level: 0

  iso:
    needs: [images-cloud, installer]
    strategy:
      fail-fast: false
      matrix:
        arch: [amd64, arm64]
        include:
          - arch: amd64
            runner: ubuntu-24.04
            grub_pkg: grub-efi-amd64-bin
          - arch: arm64
            runner: ubuntu-24.04-arm
            grub_pkg: grub-efi-arm64-bin
    runs-on: ${{ matrix.runner }}
    steps:
      - uses: actions/checkout@v6
      - uses: extractions/setup-just@v3

      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            debootstrap gdisk dosfstools e2fsprogs squashfs-tools \
            ${{ matrix.grub_pkg }} grub-common \
            parted util-linux zstd cryptsetup xorriso jq

      - name: Download cloud raw image
        uses: actions/download-artifact@v8
        with:
          name: image-raw-cloud-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/cloud/

      - name: Download installer binary
        uses: actions/download-artifact@v8
        with:
          name: installer-${{ matrix.arch }}
          path: installer-bin/

      - name: Make installer binary executable
        run: chmod +x installer-bin/bes-installer

      - name: List inputs
        run: |
          echo "=== Cloud Image ==="
          ls -lhR output/${{ matrix.arch }}/cloud/ || true
          echo "=== Installer ==="
          ls -lh installer-bin/

      - name: Build ISO
        run: |
          SOURCE_IMAGE="$(find output/${{ matrix.arch }}/cloud/ -name '*.raw.zst' | head -1)"
          if [ -z "$SOURCE_IMAGE" ]; then
            echo "ERROR: no cloud .raw.zst found under output/${{ matrix.arch }}/cloud/"
            exit 1
          fi
          sudo ARCH=${{ matrix.arch }} \
               OUTPUT=output/${{ matrix.arch }}/bes-installer-${{ matrix.arch }}.iso \
               INSTALLER_BIN=installer-bin/bes-installer \
               SOURCE_IMAGE="$SOURCE_IMAGE" \
               UBUNTU_SUITE=${{ env.UBUNTU_SUITE }} \
               iso/build-iso.sh
        timeout-minutes: 30

      - name: Test ISO structure
        run: just arch=${{ matrix.arch }} installer_bin=installer-bin/bes-installer iso-test-structure

      - name: Upload ISO
        uses: actions/upload-artifact@v7
        with:
          name: iso-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/bes-installer-*.iso
          if-no-files-found: error
          retention-days: 1
          compression-level: 0

  container-test:
    needs: [iso]
    strategy:
      fail-fast: false
      matrix:
        include:
          - arch: amd64
            runner: ubuntu-24.04
          - arch: arm64
            runner: ubuntu-24.04-arm
    runs-on: ${{ matrix.runner }}
    steps:
      - uses: actions/checkout@v6
      - uses: extractions/setup-just@v3

      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            systemd-container squashfs-tools xorriso \
            cryptsetup btrfs-progs util-linux gdisk parted

      - name: Load kernel modules
        run: |
          sudo modprobe loop
          sudo modprobe btrfs
          sudo modprobe dm-crypt

      - name: Download ISO
        uses: actions/download-artifact@v8
        with:
          name: iso-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/

      - name: List ISO
        run: ls -lh output/${{ matrix.arch }}/

      - name: Run container isolation test # r[verify installer.container.isolation]
        run: just arch=${{ matrix.arch }} variant=metal test-container-isolation
        timeout-minutes: 5

      - name: Run container install test (all scenarios, fake-LUKS auto-detected)
        run: just arch=${{ matrix.arch }} test-container-install
        timeout-minutes: 30

  release:
    needs: [images-cloud, images-metal, iso, container-test]
    if: startsWith(github.ref, 'refs/tags/')
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6

      - uses: actions/download-artifact@v8
        with:
          path: artifacts/

      - name: Prepare release assets
        run: |
          mkdir -p release

          # Copy image artifacts (raw.zst, vmdk, qcow2)
          for variant in metal cloud; do
            for arch in amd64 arm64; do
              raw_dir="artifacts/image-raw-${variant}-${arch}"
              fmt_dir="artifacts/image-formats-${variant}-${arch}"
              if [ -d "$raw_dir" ]; then
                cp "$raw_dir"/*.raw.zst release/ 2>/dev/null || true
              fi
              if [ -d "$fmt_dir" ]; then
                cp "$fmt_dir"/*.vmdk release/ 2>/dev/null || true
                cp "$fmt_dir"/*.qcow2 release/ 2>/dev/null || true
              fi
            done
          done

          # Copy ISOs
          for arch in amd64 arm64; do
            dir="artifacts/iso-${arch}"
            if [ -d "$dir" ]; then
              cp "$dir"/*.iso release/ 2>/dev/null || true
            fi
          done

          # r[image.output.checksum]
          cd release
          rm -f SHA256SUMS
          sha256sum * | tee SHA256SUMS

      - run: ls -lh release/

      - uses: softprops/action-gh-release@v2
        with:
          body: |
            ### Variants
            | Variant | Use case |
            |---------|----------|
            | metal | Install directly on hardware |
            | cloud | For cloud/VM deployments (including on-prem virtualisation) |

            ### Formats
            | Format | Use case |
            |--------|----------|
            |  iso   | Boot from USB for interactive or automated install |
            |  raw   | Write directly to server disk |
            |  vmdk  | VMware / vSphere |
            |  qcow2 | KVM / libvirt / Proxmox |
          files: release/*
          fail_on_unmatched_files: false
          make_latest: true
