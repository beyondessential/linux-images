# r[impl ci.uptodate] Keep all `uses:` actions up to date (see dependabot.yml)
name: Build Images

on:
  pull_request:
  push:
    branches: [main]
    tags: ["v*"]

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: false

permissions:
  contents: write
  id-token: write
  attestations: write

env:
  UBUNTU_MIRROR: http://us.archive.ubuntu.com/ubuntu
  UBUNTU_PORTS_MIRROR: http://ports.ubuntu.com/ubuntu-ports

jobs:
  images-cloud:
    # r[impl ci.output-suite] r[verify ci.output-suite]
    # r[impl ci.output-arch] r[verify ci.output-arch]
    # The (arch × suite) matrix produces all four combinations.
    strategy:
      fail-fast: false
      matrix:
        arch: [amd64, arm64]
        suite: [noble, resolute]
    runs-on: ${{ matrix.arch == 'amd64' && 'ubuntu-24.04' || 'ubuntu-24.04-arm' }}

    steps:
      - uses: actions/checkout@v6
      - uses: taiki-e/install-action@v2
        with:
          tool: just

      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            debootstrap gdisk dosfstools e2fsprogs btrfs-progs \
            cryptsetup parted util-linux rsync shellcheck \
            qemu-utils genisoimage zstd squashfs-tools jq

      - name: Run shellcheck # r[impl ci.shellcheck] r[verify ci.shellcheck]
        if: matrix.suite == 'noble'
        run: just test-shellcheck

      - name: Build raw image
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} variant=cloud raw
        timeout-minutes: 60

      - name: Test image structure
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} variant=cloud test-structure

      - name: Produce final artifacts
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} variant=cloud build
        timeout-minutes: 30

      - name: Verify outputs # r[verify image.output.raw] r[verify image.output.vmdk] r[verify image.output.qcow2] r[verify image.output.checksum]
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} variant=cloud verify-outputs

      - name: List outputs
        run: ls -lh output/${{ matrix.arch }}/cloud/

      - name: Upload raw image (needed by ISO build)
        uses: actions/upload-artifact@v7
        with:
          name: image-raw-cloud-${{ matrix.suite }}-${{ matrix.arch }}
          path: |
            output/${{ matrix.arch }}/cloud/*.raw.zst
            output/${{ matrix.arch }}/cloud/*.raw.size
          if-no-files-found: error
          retention-days: 1
          archive: false

      # Per-format uploads: each artifact is a single file, so archive: false
      # works (it requires exactly one file).
      - name: Upload VMDK (needed by release)
        uses: actions/upload-artifact@v7
        with:
          name: image-vmdk-cloud-${{ matrix.suite }}-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/cloud/*.vmdk
          if-no-files-found: error
          retention-days: 1
          archive: false

      - name: Upload qcow2 (needed by release)
        uses: actions/upload-artifact@v7
        with:
          name: image-qcow2-cloud-${{ matrix.suite }}-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/cloud/*.qcow2
          if-no-files-found: error
          retention-days: 1
          archive: false

  images-metal:
    strategy:
      fail-fast: false
      matrix:
        arch: [amd64, arm64]
        suite: [noble, resolute]
    runs-on: ${{ matrix.arch == 'amd64' && 'ubuntu-24.04' || 'ubuntu-24.04-arm' }}

    steps:
      - uses: actions/checkout@v6
      - uses: taiki-e/install-action@v2
        with:
          tool: just

      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            debootstrap gdisk dosfstools e2fsprogs btrfs-progs \
            cryptsetup parted util-linux rsync \
            qemu-utils genisoimage zstd squashfs-tools jq

      - name: Build raw image
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} variant=metal raw
        timeout-minutes: 60

      - name: Test image structure
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} variant=metal test-structure

      - name: Produce final artifacts
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} variant=metal build
        timeout-minutes: 30

      - name: Verify outputs # r[verify image.output.raw] r[verify image.output.vmdk] r[verify image.output.qcow2] r[verify image.output.checksum]
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} variant=metal verify-outputs

      - name: List outputs
        run: ls -lh output/${{ matrix.arch }}/metal/

      - name: Upload raw image (needed by release)
        uses: actions/upload-artifact@v7
        with:
          name: image-raw-metal-${{ matrix.suite }}-${{ matrix.arch }}
          path: |
            output/${{ matrix.arch }}/metal/*.raw.zst
            output/${{ matrix.arch }}/metal/*.raw.size
          if-no-files-found: error
          retention-days: 1
          archive: false

      - name: Upload VMDK (needed by release)
        uses: actions/upload-artifact@v7
        with:
          name: image-vmdk-metal-${{ matrix.suite }}-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/metal/*.vmdk
          if-no-files-found: error
          retention-days: 1
          archive: false

      - name: Upload qcow2 (needed by release)
        uses: actions/upload-artifact@v7
        with:
          name: image-qcow2-metal-${{ matrix.suite }}-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/metal/*.qcow2
          if-no-files-found: error
          retention-days: 1
          archive: false

  iso:
    needs: [images-cloud]
    strategy:
      fail-fast: false
      matrix:
        arch: [amd64, arm64]
        suite: [noble, resolute]
    # r[impl ci.installer-target] r[verify ci.installer-target]
    # The installer links against the runner's glibc, which must be <= the
    # glibc in the live ISO rootfs. ubuntu-24.04 (glibc 2.39) targets both
    # noble (2.39) and resolute (≥2.41) cleanly. When bumping the runner
    # image, verify its glibc does not exceed the lowest target suite's
    # glibc before merging.
    runs-on: ${{ matrix.arch == 'amd64' && 'ubuntu-24.04' || 'ubuntu-24.04-arm' }}
    steps:
      - uses: actions/checkout@v6
      - uses: taiki-e/install-action@v2
        with:
          tool: just

      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            debootstrap gdisk dosfstools e2fsprogs squashfs-tools \
            grub-efi-${{ matrix.arch }}-bin grub-common \
            parted util-linux zstd cryptsetup xorriso jq

      # r[impl ci.rust-stable] r[verify ci.rust-stable]
      - name: Install Rust toolchain via rustup
        run: |
          rustup update stable
          rustup default stable
          rustup target add ${{ matrix.arch == 'amd64' && 'x86_64-unknown-linux-gnu' || 'aarch64-unknown-linux-gnu' }}

      # r[impl ci.rust-cache] r[verify ci.rust-cache]
      - uses: Swatinem/rust-cache@v2

      - name: Download cloud raw image
        uses: actions/download-artifact@v8
        with:
          name: image-raw-cloud-${{ matrix.suite }}-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/cloud/

      - name: List inputs
        run: |
          echo "=== Cloud Image ==="
          ls -lhR output/${{ matrix.arch }}/cloud/ || true

      - name: Build ISO
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} iso
        timeout-minutes: 30

      - name: Test ISO structure
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} iso-test-structure

      - name: Upload ISO
        uses: actions/upload-artifact@v7
        with:
          name: iso-${{ matrix.suite }}-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/bes-installer-*.iso
          if-no-files-found: error
          retention-days: 1
          archive: false

  container-test:
    needs: [iso]
    strategy:
      fail-fast: false
      matrix:
        arch: [amd64, arm64]
        suite: [noble, resolute]
    runs-on: ${{ matrix.arch == 'amd64' && 'ubuntu-24.04' || 'ubuntu-24.04-arm' }}
    steps:
      - uses: actions/checkout@v6
      - uses: taiki-e/install-action@v2
        with:
          tool: just

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
          name: iso-${{ matrix.suite }}-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/

      - name: List ISO
        run: ls -lh output/${{ matrix.arch }}/

      - name: Run container isolation test # r[verify installer.container.isolation]
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} variant=metal test-container-isolation
        timeout-minutes: 5

      - name: Run container install test (all scenarios, fake-LUKS auto-detected)
        run: just ubuntu_suite=${{ matrix.suite }} arch=${{ matrix.arch }} test-container-install
        timeout-minutes: 30

  all-green:
    name: All builds green
    if: always()
    needs: [images-cloud, images-metal, iso, container-test]
    runs-on: ubuntu-latest
    steps:
      - name: Check job results
        run: |
          result='${{ toJSON(needs) }}'
          echo "$result" | jq .
          echo "$result" | jq -e 'all(.result == "success")'

  release:
    needs: [images-cloud, images-metal, iso, container-test]
    if: startsWith(github.ref, 'refs/tags/')
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6

      - uses: actions/download-artifact@v8
        with:
          path: artifacts/

      - name: Derive version from tag
        run: echo "VERSION=${GITHUB_REF_NAME#v}" >> "$GITHUB_ENV"

      - name: Prepare release assets
        run: |
          mkdir -p release

          # Copy image artifacts (raw.zst, vmdk, qcow2). Each format lives
          # in its own artifact dir.
          for variant in metal cloud; do
            for suite in noble resolute; do
              for arch in amd64 arm64; do
                raw_dir="artifacts/image-raw-${variant}-${suite}-${arch}"
                vmdk_dir="artifacts/image-vmdk-${variant}-${suite}-${arch}"
                qcow_dir="artifacts/image-qcow2-${variant}-${suite}-${arch}"
                [ -d "$raw_dir" ]  && cp "$raw_dir"/*.raw.zst release/ 2>/dev/null || true
                [ -d "$vmdk_dir" ] && cp "$vmdk_dir"/*.vmdk release/ 2>/dev/null || true
                [ -d "$qcow_dir" ] && cp "$qcow_dir"/*.qcow2 release/ 2>/dev/null || true
              done
            done
          done

          # Copy ISOs
          for suite in noble resolute; do
            for arch in amd64 arm64; do
              dir="artifacts/iso-${suite}-${arch}"
              if [ -d "$dir" ]; then
                cp "$dir"/*.iso release/ 2>/dev/null || true
              fi
            done
          done

          # r[image.output.checksum]
          cd release
          rm -f SHA256SUMS
          sha256sum * | tee SHA256SUMS

      - name: Generate manifest.json
        run: |
          cd release
          jq -n \
            --arg version "$VERSION" \
            --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
            --arg repo "https://github.com/${{ github.repository }}" \
            --arg tag "${{ github.ref_name }}" \
            '{ version: $version, date: $date, repo: $repo, tag: $tag, files: [] }' \
            > manifest.json

          for f in *; do
            [ "$f" = "manifest.json" ] && continue
            [ "$f" = "index.html" ] && continue

            size=$(stat --format='%s' "$f")
            sha256=$(grep " ${f}\$" SHA256SUMS | cut -d' ' -f1 || true)

            variant=""
            arch=""
            version=""
            suite=""
            format="$f"

            case "$f" in
              ubuntu-*-bes-*-*-*.raw.zst)
                version=$(echo "$f" | sed -E 's/ubuntu-([^-]+)-bes-.*$/\1/')
                variant=$(echo "$f" | sed -E 's/ubuntu-[^-]+-bes-([^-]+)-.*$/\1/')
                arch=$(echo "$f" | sed -E 's/ubuntu-[^-]+-bes-[^-]+-([^-]+)-.*$/\1/')
                format="raw.zst"
                ;;
              ubuntu-*-bes-*-*-*.vmdk)
                version=$(echo "$f" | sed -E 's/ubuntu-([^-]+)-bes-.*$/\1/')
                variant=$(echo "$f" | sed -E 's/ubuntu-[^-]+-bes-([^-]+)-.*$/\1/')
                arch=$(echo "$f" | sed -E 's/ubuntu-[^-]+-bes-[^-]+-([^-]+)-.*$/\1/')
                format="vmdk"
                ;;
              ubuntu-*-bes-*-*-*.qcow2)
                version=$(echo "$f" | sed -E 's/ubuntu-([^-]+)-bes-.*$/\1/')
                variant=$(echo "$f" | sed -E 's/ubuntu-[^-]+-bes-([^-]+)-.*$/\1/')
                arch=$(echo "$f" | sed -E 's/ubuntu-[^-]+-bes-[^-]+-([^-]+)-.*$/\1/')
                format="qcow2"
                ;;
              bes-installer-*-*.iso)
                version=$(echo "$f" | sed -E 's/bes-installer-([^-]+)-.*\.iso/\1/')
                variant="installer"
                arch=$(echo "$f" | sed -E 's/bes-installer-[^-]+-([^.]+)\.iso/\1/')
                format="iso"
                ;;
              SHA256SUMS)
                format="checksums"
                ;;
            esac

            # Map ubuntu_version → suite codename. Keep this in lockstep
            # with the justfile mapping.
            case "$version" in
              24.04) suite="noble" ;;
              26.04) suite="resolute" ;;
            esac

            manifest_entry=$(jq -n \
              --arg name "$f" \
              --argjson size "$size" \
              --arg sha256 "$sha256" \
              --arg variant "$variant" \
              --arg arch "$arch" \
              --arg suite "$suite" \
              --arg format "$format" \
              '{ name: $name, size: $size, sha256: $sha256, variant: $variant, arch: $arch, suite: $suite, format: $format }
               | del(.[] | select(. == ""))')

            jq --argjson entry "$manifest_entry" '.files += [$entry]' manifest.json > manifest.tmp
            mv manifest.tmp manifest.json
          done

      - name: Generate index.html
        run: |
          cd release
          cat > index.html <<'HTMLEOF'
          <!DOCTYPE html>
          <html lang="en">
          <head>
          <meta charset="utf-8">
          <meta name="viewport" content="width=device-width, initial-scale=1">
          <title>BES Linux Images — VERSION_PLACEHOLDER</title>
          <style>
            body { font-family: system-ui, -apple-system, sans-serif; max-width: 52rem; margin: 2rem auto; padding: 0 1rem; color: #1a1a1a; }
            h1 { font-size: 1.4rem; }
            a { color: #0060df; }
            table { border-collapse: collapse; width: 100%; margin: 1.5rem 0; }
            th, td { text-align: left; padding: 0.4rem 0.8rem; border-bottom: 1px solid #ddd; }
            th { font-weight: 600; border-bottom: 2px solid #999; }
            td.size { text-align: right; font-variant-numeric: tabular-nums; }
            code { font-size: 0.85em; background: #f0f0f0; padding: 0.1em 0.3em; border-radius: 3px; }
            .meta { color: #555; font-size: 0.9rem; margin-bottom: 1.5rem; }
          </style>
          </head>
          <body>
          <h1>BES Linux Images &mdash; VERSION_PLACEHOLDER</h1>
          <p class="meta">
            Source: <a href="REPO_PLACEHOLDER">REPO_PLACEHOLDER</a>
            &middot; Tag: <a href="REPO_PLACEHOLDER/releases/tag/TAG_PLACEHOLDER"><code>TAG_PLACEHOLDER</code></a>
            &middot; <a href="manifest.json">manifest.json</a>
          </p>
          <table>
          <thead><tr><th>File</th><th>Variant</th><th>Suite</th><th>Arch</th><th>Format</th><th class="size">Size</th></tr></thead>
          <tbody>
          TABLE_ROWS_PLACEHOLDER
          </tbody>
          </table>
          </body>
          </html>
          HTMLEOF

          repo="https://github.com/${{ github.repository }}"
          tag="${{ github.ref_name }}"

          human_size() {
            local bytes=$1
            if [ "$bytes" -ge 1073741824 ]; then
              awk "BEGIN { printf \"%.1f GiB\", $bytes/1073741824 }"
            elif [ "$bytes" -ge 1048576 ]; then
              awk "BEGIN { printf \"%.1f MiB\", $bytes/1048576 }"
            else
              awk "BEGIN { printf \"%.0f KiB\", $bytes/1024 }"
            fi
          }

          rows=""
          jq -c '.files[]' manifest.json | while IFS= read -r entry; do
            name=$(echo "$entry" | jq -r '.name')
            size=$(echo "$entry" | jq -r '.size')
            variant=$(echo "$entry" | jq -r '.variant // ""')
            suite=$(echo "$entry" | jq -r '.suite // ""')
            arch=$(echo "$entry" | jq -r '.arch // ""')
            format=$(echo "$entry" | jq -r '.format')
            hsize=$(human_size "$size")
            echo "<tr><td><a href=\"${name}\">${name}</a></td><td>${variant}</td><td>${suite}</td><td>${arch}</td><td>${format}</td><td class=\"size\">${hsize}</td></tr>"
          done > table_rows.tmp

          sed -i "s|VERSION_PLACEHOLDER|${VERSION}|g" index.html
          sed -i "s|REPO_PLACEHOLDER|${repo}|g" index.html
          sed -i "s|TAG_PLACEHOLDER|${tag}|g" index.html
          sed -i "/TABLE_ROWS_PLACEHOLDER/{
            r table_rows.tmp
            d
          }" index.html
          rm -f table_rows.tmp

      - run: ls -lh release/

      - name: Attest build provenance
        uses: actions/attest-build-provenance@v2
        with:
          subject-path: release/*

      - name: Configure AWS Credentials
        uses: aws-actions/configure-aws-credentials@v4
        with:
          aws-region: ap-southeast-2
          role-to-assume: arn:aws:iam::143295493206:role/gha-linux-images-upload
          role-session-name: GHA@linux-images=Release

      - name: Upload to S3
        run: |
          for f in release/*; do
            name=$(basename "$f")
            content_type=""
            case "$name" in
              index.html)     content_type="text/html" ;;
              manifest.json)  content_type="application/json" ;;
              SHA256SUMS)     content_type="text/plain" ;;
            esac
            if [ -n "$content_type" ]; then
              aws s3 cp "$f" "s3://bes-ops-tools/linux-images/${{ env.VERSION }}/$name" --no-progress --content-type "$content_type"
            else
              aws s3 cp "$f" "s3://bes-ops-tools/linux-images/${{ env.VERSION }}/$name" --no-progress
            fi
          done

      - name: Invalidate CloudFront cache
        run: aws cloudfront create-invalidation --distribution-id=EDAG0UBS1MN74 --paths '/linux-images/*'

      - uses: softprops/action-gh-release@v2
        with:
          body: |
            **Downloads: <https://tools.ops.tamanu.io/linux-images/${{ env.VERSION }}/>**

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
          files: release/SHA256SUMS
          fail_on_unmatched_files: true
          make_latest: true

  register-ami:
    needs: [images-cloud]
    if: startsWith(github.ref, 'refs/tags/')
    strategy:
      fail-fast: false
      matrix:
        arch: [amd64, arm64]
        suite: [noble, resolute]
    runs-on: ${{ matrix.arch == 'amd64' && 'ubuntu-24.04' || 'ubuntu-24.04-arm' }}
    steps:
      - uses: actions/checkout@v6

      - name: Derive version from tag
        run: echo "VERSION=${GITHUB_REF_NAME#v}" >> "$GITHUB_ENV"

      - name: Install zstd
        run: sudo apt-get install -y --no-install-recommends zstd

      - name: Download cloud raw image
        uses: actions/download-artifact@v8
        with:
          name: image-raw-cloud-${{ matrix.suite }}-${{ matrix.arch }}
          path: output/${{ matrix.arch }}/cloud/

      - name: Configure AWS Credentials
        uses: aws-actions/configure-aws-credentials@v4
        with:
          aws-region: ap-southeast-2
          role-to-assume: arn:aws:iam::143295493206:role/gha-linux-images-upload
          role-session-name: GHA@linux-images=RegisterAMI-${{ matrix.suite }}-${{ matrix.arch }}

      - name: Register AMI
        run: scripts/register-ami-for-release.sh "${{ matrix.arch }}" "${{ env.VERSION }}"
        timeout-minutes: 60
