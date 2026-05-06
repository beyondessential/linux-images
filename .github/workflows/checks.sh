# r[impl ci.uptodate] Keep all `uses:` actions up to date (see dependabot.yml)
name: Checks

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
  rust-test:
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

  rust-lint:
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

  tracey:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6

      - name: Install tracey
        run: curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bearcove/tracey/releases/download/v1.3.0/tracey-installer.sh | sh

      # tracey can't parse YAML, so we commit .sh copies of .yml files.
      # Verify the copies are up to date (run `just tracey-setup` to fix).
      - name: Check .yml/.sh copies are in sync
        run: |
          FAILED=0
          for f in .github/workflows/*.yml .github/*.yml; do
            sh="${f%.yml}.sh"
            if [ ! -f "$sh" ]; then
              echo "::error::Missing $sh (copy of $f). Run 'just tracey-setup' and commit."
              FAILED=1
            elif ! diff -q "$f" "$sh" >/dev/null 2>&1; then
              echo "::error::$sh is out of date with $f. Run 'just tracey-setup' and commit."
              FAILED=1
            fi
          done
          if [ "$FAILED" -ne 0 ]; then exit 1; fi
          echo "All .sh copies are in sync with .yml sources."

      - name: Start tracey web server
        run: |
          tracey web --port 3199 &
          for i in $(seq 1 30); do
            if curl -sf http://localhost:3199/api/status >/dev/null 2>&1; then
              echo "tracey web server ready after ${i}s"
              exit 0
            fi
            sleep 1
          done
          echo "::error::tracey web server failed to start within 30s"
          exit 1

      - name: Validate spec
        run: |
          RESULT="$(curl -sf http://localhost:3199/api/validate)"
          echo "$RESULT" | jq .
          ERRORS="$(echo "$RESULT" | jq '.errorCount')"
          if [ "$ERRORS" -ne 0 ]; then
            echo "::error::tracey validate found $ERRORS error(s)"
            exit 1
          fi

      - name: Check coverage status
        run: |
          RESULT="$(curl -sf http://localhost:3199/api/status)"
          echo "$RESULT" | jq .
          TOTAL="$(echo "$RESULT" | jq '.impls[0].totalRules')"
          COVERED="$(echo "$RESULT" | jq '.impls[0].coveredRules')"
          VERIFIED="$(echo "$RESULT" | jq '.impls[0].verifiedRules')"
          STALE="$(echo "$RESULT" | jq '.impls[0].staleRules')"
          echo "Coverage: $COVERED/$TOTAL implemented, $VERIFIED/$TOTAL verified, $STALE stale"

          FAILED=0

          if [ "$STALE" -ne 0 ]; then
            echo "::error::$STALE stale reference(s) found"
            FAILED=1
          fi

          if [ "$COVERED" -ne "$TOTAL" ]; then
            UNCOVERED="$(curl -sf http://localhost:3199/api/uncovered)"
            echo "$UNCOVERED" | jq '.bySection[] | .section as $s | .rules[] | "\($s): \(.id.base)"'
            echo "::error::Only $COVERED/$TOTAL rules have implementation references"
            FAILED=1
          fi

          if [ "$VERIFIED" -ne "$TOTAL" ]; then
            UNTESTED="$(curl -sf http://localhost:3199/api/untested)"
            echo "$UNTESTED" | jq '.bySection[] | .section as $s | .rules[] | "\($s): \(.id.base)"'
            echo "::error::Only $VERIFIED/$TOTAL rules have verification references"
            FAILED=1
          fi

          if [ "$FAILED" -ne 0 ]; then
            exit 1
          fi

      - name: Summary
        if: always()
        run: |
          STATUS="$(curl -sf http://localhost:3199/api/status 2>/dev/null || echo '{}')"
          TOTAL="$(echo "$STATUS" | jq '.impls[0].totalRules // 0')"
          COVERED="$(echo "$STATUS" | jq '.impls[0].coveredRules // 0')"
          VERIFIED="$(echo "$STATUS" | jq '.impls[0].verifiedRules // 0')"
          STALE="$(echo "$STATUS" | jq '.impls[0].staleRules // 0')"

          {
            echo "## Tracey Spec Coverage"
            echo ""
            echo "| Metric | Count |"
            echo "|--------|-------|"
            echo "| Total rules | $TOTAL |"
            echo "| Implemented | $COVERED |"
            echo "| Verified | $VERIFIED |"
            echo "| Stale | $STALE |"
          } >> "$GITHUB_STEP_SUMMARY"

  manual-docs:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6

      - name: Check generated files are up to date
        run: scripts/gen-manual-testing.sh --check

  all-green:
    name: All green
    if: always()
    needs: [rust-test, rust-lint, tracey, manual-docs]
    runs-on: ubuntu-latest
    steps:
      - name: Check job results
        run: |
          result='${{ toJSON(needs) }}'
          echo "$result" | jq .
          echo "$result" | jq -e 'all(.result == "success")'
