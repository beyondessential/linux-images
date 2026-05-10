#!/bin/bash
# Verify the Pi 5 EEPROM-config SD artifact.
#
# Usage: test-pi-eeprom-sd.sh <output-dir> [<image-path>]
#
# - <output-dir> must contain recovery.bin, pieeprom.upd, pieeprom.sig.
# - If <image-path> is given and exists, also checks the partition layout
#   and that the FAT contains the same three files.

set -euo pipefail

OUTPUT_DIR="${1:?usage: $0 <output-dir> [<image-path>]}"
IMAGE_PATH="${2:-}"

PASS=0
FAIL=0

check() {
    local desc="$1"; shift
    if "$@"; then
        echo "PASS: $desc"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $desc"
        FAIL=$((FAIL + 1))
    fi
}

# r[verify image.pi-eeprom-sd.artifact]
check "recovery.bin exists" test -f "$OUTPUT_DIR/recovery.bin"
check "pieeprom.upd exists" test -f "$OUTPUT_DIR/pieeprom.upd"
check "pieeprom.sig exists" test -f "$OUTPUT_DIR/pieeprom.sig"

# Pi 5 EEPROM is 2 MiB exactly.
PIEEPROM_SIZE=$(stat -c %s "$OUTPUT_DIR/pieeprom.upd" 2>/dev/null || echo 0)
check "pieeprom.upd is 2 MiB" test "$PIEEPROM_SIZE" -eq 2097152

SIG_SHA="$(sed -n 1p "$OUTPUT_DIR/pieeprom.sig" 2>/dev/null || true)"
SIG_TS="$(sed -n 2p "$OUTPUT_DIR/pieeprom.sig" 2>/dev/null || true)"
ACTUAL_SHA="$(sha256sum "$OUTPUT_DIR/pieeprom.upd" | awk '{print $1}')"
check "pieeprom.sig line 1 is sha256(pieeprom.upd)" test "$SIG_SHA" = "$ACTUAL_SHA"
check "pieeprom.sig line 2 starts with 'ts: <digits>'" \
    bash -c '[[ "'"$SIG_TS"'" =~ ^ts:\ [0-9]+$ ]]'

# r[verify image.pi-eeprom-sd.bootconf]
# Settings are stored as plain text inside the EEPROM blob, so we can grep
# strings(1)-style via plain bytes search. Each setting must appear as
# `KEY=VALUE`.
expect_setting() {
    local kv="$1"
    grep -aFq "$kv" "$OUTPUT_DIR/pieeprom.upd"
}
check "BOOT_UART=1 baked in"            expect_setting BOOT_UART=1
check "BOOT_ORDER=0xf61 baked in"       expect_setting BOOT_ORDER=0xf61
check "POWER_OFF_ON_HALT=1 baked in"    expect_setting POWER_OFF_ON_HALT=1
check "PCIE_PROBE=1 baked in"           expect_setting PCIE_PROBE=1
check "HDMI_DELAY=0 baked in"           expect_setting HDMI_DELAY=0
check "PSU_MAX_CURRENT=5000 baked in"   expect_setting PSU_MAX_CURRENT=5000

if [ -n "$IMAGE_PATH" ] && [ -f "$IMAGE_PATH" ]; then
    IMG_SIZE=$(stat -c %s "$IMAGE_PATH")
    check ".img is at most 64 MiB" test "$IMG_SIZE" -le $((64 * 1024 * 1024))

    PART_INFO="$(sfdisk -d "$IMAGE_PATH" 2>/dev/null || true)"
    check ".img has DOS partition table" \
        bash -c '[[ "'"$PART_INFO"'" == *"label: dos"* ]]'
    check ".img has exactly one partition" \
        bash -c '[[ "$(echo "'"$PART_INFO"'" | grep -c "^/.*: start")" == "1" || "$(echo "'"$PART_INFO"'" | grep -c "^.*img1 :")" == "1" ]]'
    check ".img partition is FAT16 (type 6)" \
        bash -c '[[ "'"$PART_INFO"'" == *"type=6"* ]]'

    if command -v mdir >/dev/null 2>&1; then
        # mtools accepts an offset into the image directly via the @@ syntax.
        # The partition starts at sector 2048 = 1 MiB = 1048576 bytes.
        FAT_LISTING="$(MTOOLS_SKIP_CHECK=1 mdir -i "${IMAGE_PATH}@@1048576" -b ::/ 2>/dev/null || true)"
        check ".img FAT contains recovery.bin" \
            bash -c '[[ "'"$FAT_LISTING"'" == *"RECOVERY.BIN"* || "'"$FAT_LISTING"'" == *"recovery.bin"* ]]'
        check ".img FAT contains pieeprom.upd" \
            bash -c '[[ "'"$FAT_LISTING"'" == *"PIEEPROM.UPD"* || "'"$FAT_LISTING"'" == *"pieeprom.upd"* ]]'
        check ".img FAT contains pieeprom.sig" \
            bash -c '[[ "'"$FAT_LISTING"'" == *"PIEEPROM.SIG"* || "'"$FAT_LISTING"'" == *"pieeprom.sig"* ]]'

    else
        echo "SKIP: mtools not installed — skipping FAT contents check"
    fi

    # Read the FAT16 volume label straight from the boot-sector BPB at
    # partition offset 43 (11 bytes, space-padded). Independent of mtools
    # version. Partition starts at 1 MiB, so absolute offset is 1048619.
    VOL_LABEL="$(dd if="$IMAGE_PATH" bs=1 skip=1048619 count=11 status=none 2>/dev/null | tr -d '\000' | sed 's/ *$//')"
    check ".img volume label is RECOVERY" test "$VOL_LABEL" = "RECOVERY"
fi

echo ""
echo "RESULTS: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
