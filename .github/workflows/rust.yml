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
          # r[impl ci.installer-target] r[verify ci.installer-target]
          - runner: ubuntu-24.04
            cargo_target: x86_64-unknown-linux-gnu
          - runner: ubuntu-24.04-arm
            cargo_target: aarch64-unknown-linux-gnu
    runs-on: ${{ matrix.runner }}
    steps:
      - uses: actions/checkout@v6

      # r[impl ci.rust-stable] r[verify ci.rust-stable]
      - run: |
          rustup update stable
          rustup default stable
          rustup target add ${{ matrix.cargo_target }}

      # r[impl ci.rust-cache] r[verify ci.rust-cache]
      - uses: Swatinem/rust-cache@v2

      # r[impl ci.unit-test] r[verify ci.unit-test]
      - run: cargo test -p bes-installer

      - name: Build release binary
        run: cargo build --release --target ${{ matrix.cargo_target }} -p bes-installer

      - name: Verify binary is dynamically linked against glibc
        run: |
          file target/${{ matrix.cargo_target }}/release/bes-installer
          ldd target/${{ matrix.cargo_target }}/release/bes-installer | grep -q libc.so || \
            echo "::warning::Binary does not appear to link against glibc"

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
