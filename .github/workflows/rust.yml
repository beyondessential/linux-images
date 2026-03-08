# r[impl ci.uptodate] Keep all `uses:` actions up to date (see dependabot.yml)
name: Rust checks

on:
  pull_request:
  push:
    branches: [main]

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read

jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        include:
          - runner: ubuntu-24.04
            cargo_target: x86_64-unknown-linux-musl
          - runner: ubuntu-24.04-arm
            cargo_target: aarch64-unknown-linux-musl
    runs-on: ${{ matrix.runner }}
    steps:
      - uses: actions/checkout@v6

      # r[impl ci.rust-stable] r[verify ci.rust-stable]
      - run: |
          rustup update stable
          rustup default stable
          rustup target add ${{ matrix.cargo_target }}

      - name: Install musl tools
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends musl-tools

      # r[impl ci.rust-cache] r[verify ci.rust-cache]
      - uses: Swatinem/rust-cache@v2

      # r[impl ci.unit-test] r[verify ci.unit-test]
      - run: cargo test -p bes-installer

      - name: Build release binary (static musl)
        run: cargo build --release --target ${{ matrix.cargo_target }} -p bes-installer

      - name: Verify binary is static
        run: |
          file target/${{ matrix.cargo_target }}/release/bes-installer
          ldd target/${{ matrix.cargo_target }}/release/bes-installer 2>&1 | grep -q "not a dynamic" || \
            ldd target/${{ matrix.cargo_target }}/release/bes-installer 2>&1 | grep -q "statically linked" || \
            echo "::warning::Binary may not be fully static"

  lint:
    strategy:
      fail-fast: false
      matrix:
        runner: [ubuntu-24.04, ubuntu-24.04-arm]
    runs-on: ${{ matrix.runner }}
    steps:
      - uses: actions/checkout@v6

      # r[impl ci.rust-stable] r[verify ci.rust-stable]
      - run: |
          rustup update stable
          rustup default stable

      # r[impl ci.rust-cache] r[verify ci.rust-cache]
      - uses: Swatinem/rust-cache@v2

      - run: cargo clippy -- -D warnings
      - run: cargo fmt --check
