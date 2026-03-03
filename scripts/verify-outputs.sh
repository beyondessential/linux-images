#!/bin/bash
# Verify that all expected output formats and checksums are present and valid.
# Usage: verify-outputs.sh <output_dir> <filestem>
set -euo pipefail

OUTPUT_DIR="${1:?Usage: $0 <output_dir> <filestem>}"
FILESTEM="${2:?Usage: $0 <output_dir> <filestem>}"

PASS=0
FAIL=0
ERRORS=()

pass() {
    local desc="$1"
    echo "  PASS: $desc"
    PASS=$((PASS + 1))
}

fail() {
    local desc="$1"
    echo "  FAIL: $desc"
    ERRORS+=("$desc")
    FAIL=$((FAIL + 1))
}

check() {
    local desc="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        pass "$desc"
    else
        fail "$desc"
    fi
}

echo "=============================="
echo "Output Verification"
echo "=============================="
echo "Directory: $OUTPUT_DIR"
echo "Filestem:  $FILESTEM"
echo "=============================="
echo ""

# --- Expected files ---

RAW_ZST="$OUTPUT_DIR/${FILESTEM}.raw.zst"
VMDK="$OUTPUT_DIR/${FILESTEM}.vmdk"
QCOW2="$OUTPUT_DIR/${FILESTEM}.qcow2"
SHA256SUMS="$OUTPUT_DIR/SHA256SUMS"

# r[verify image.output.raw]
echo "--- Compressed raw image ---"
check "compressed raw image exists (${FILESTEM}.raw.zst)" test -f "$RAW_ZST"
if [ -f "$RAW_ZST" ]; then
    SIZE=$(stat -c%s "$RAW_ZST")
    check "compressed raw image is non-empty" test "$SIZE" -gt 0
    check "compressed raw image is valid zstd" zstd -t "$RAW_ZST"
fi

# r[verify image.output.vmdk]
echo "--- VMDK image ---"
check "VMDK image exists (${FILESTEM}.vmdk)" test -f "$VMDK"
if [ -f "$VMDK" ]; then
    SIZE=$(stat -c%s "$VMDK")
    check "VMDK image is non-empty" test "$SIZE" -gt 0
    check "VMDK image is recognized by qemu-img" qemu-img info "$VMDK"
    FORMAT=$(qemu-img info --output=json "$VMDK" 2>/dev/null | grep '"format"' | sed 's/.*: "\(.*\)".*/\1/' || true)
    check "VMDK format is vmdk" test "$FORMAT" = "vmdk"
fi

# r[verify image.output.qcow2]
echo "--- qcow2 image ---"
check "qcow2 image exists (${FILESTEM}.qcow2)" test -f "$QCOW2"
if [ -f "$QCOW2" ]; then
    SIZE=$(stat -c%s "$QCOW2")
    check "qcow2 image is non-empty" test "$SIZE" -gt 0
    check "qcow2 image is recognized by qemu-img" qemu-img info "$QCOW2"
    FORMAT=$(qemu-img info --output=json "$QCOW2" 2>/dev/null | grep '"format"' | sed 's/.*: "\(.*\)".*/\1/' || true)
    check "qcow2 format is qcow2" test "$FORMAT" = "qcow2"
fi

# r[verify image.output.checksum]
echo "--- SHA256SUMS ---"
check "SHA256SUMS file exists" test -f "$SHA256SUMS"
if [ -f "$SHA256SUMS" ]; then
    check "SHA256SUMS is non-empty" test -s "$SHA256SUMS"

    # Verify that each expected output has an entry in SHA256SUMS
    for artifact in "${FILESTEM}.raw.zst" "${FILESTEM}.vmdk" "${FILESTEM}.qcow2"; do
        if grep -q "$artifact" "$SHA256SUMS"; then
            pass "SHA256SUMS contains entry for $artifact"
        else
            fail "SHA256SUMS contains entry for $artifact"
        fi
    done

    # Verify the checksums actually match
    echo "--- Checksum verification ---"
    if (cd "$OUTPUT_DIR" && sha256sum --check --strict SHA256SUMS); then
        pass "all SHA256 checksums verify"
    else
        fail "all SHA256 checksums verify"
    fi
fi

# --- Ensure no uncompressed raw image is left behind ---
RAW="$OUTPUT_DIR/${FILESTEM}.raw"
if [ -f "$RAW" ]; then
    fail "uncompressed raw image should not be present after build"
else
    pass "uncompressed raw image is not present (compressed with zstd)"
fi

# --- Results ---
echo ""
echo "=============================="
echo "RESULTS: $PASS passed, $FAIL failed"
echo "=============================="

if [ $FAIL -gt 0 ]; then
    echo ""
    echo "Failures:"
    for e in "${ERRORS[@]}"; do
        echo "  - $e"
    done
    echo ""
    exit 1
fi

exit 0
