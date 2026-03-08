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

      # r[impl ci.unit-test] r[verify ci.unit-test]
      - run: cargo test -p bes-installer

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
