#!/bin/bash
set -euo pipefail

# Publish a release shard's artifacts to S3 and emit a manifest fragment.
#
# Called from each producer job's tag-gated tail. The producer already
# knows the file's (variant, arch, suite) tuple, so we don't reparse
# filenames here — those go straight into the manifest entry.
#
# Usage: publish-release-shard.sh <variant> <arch> <suite> <source-dir>
#
# variant: cloud | metal | pi | installer | pi-eeprom
# arch:    amd64 | arm64 | "" (pi-eeprom has no arch)
# suite:   noble | resolute | "" (pi-eeprom has no suite)
# source-dir: directory containing the files to publish
#
# Environment (must be set by the workflow):
#   GITHUB_REF_NAME — release tag (e.g. v2026.05.13); leading "v" is stripped
#   S3_BUCKET       — target bucket
#   S3_PREFIX       — path prefix under the bucket (e.g. "linux-images")
#
# Writes ./manifest-fragment.json with one entry per uploaded file.

VARIANT="${1:-}"
ARCH="${2:-}"
SUITE="${3:-}"
SOURCE_DIR="${4:-}"

if [ -z "$VARIANT" ] || [ -z "$SOURCE_DIR" ]; then
  echo "Usage: $0 <variant> <arch> <suite> <source-dir>" >&2
  exit 1
fi

: "${GITHUB_REF_NAME:?must be set}"
: "${S3_BUCKET:?must be set}"
: "${S3_PREFIX:?must be set}"

VERSION="${GITHUB_REF_NAME#v}"
FRAGMENT="manifest-fragment.json"
echo '[]' > "$FRAGMENT"

shopt -s nullglob

case "$VARIANT" in
  cloud|metal)
    paths=( "$SOURCE_DIR"/*.img.zst "$SOURCE_DIR"/*.vmdk "$SOURCE_DIR"/*.qcow2 )
    ;;
  pi)
    paths=( "$SOURCE_DIR"/*.img.zst )
    ;;
  installer)
    paths=( "$SOURCE_DIR"/bes-installer-*.iso )
    ;;
  pi-eeprom)
    paths=( "$SOURCE_DIR"/bes-pi-eeprom-config.img.zst )
    ;;
  *)
    echo "Unknown variant: $VARIANT" >&2
    exit 1
    ;;
esac

if [ "${#paths[@]}" -eq 0 ]; then
  echo "No files matched for variant=$VARIANT in $SOURCE_DIR" >&2
  exit 1
fi

for path in "${paths[@]}"; do
  name="$(basename "$path")"
  sha="$(sha256sum "$path" | cut -d' ' -f1)"
  size="$(stat --format='%s' "$path")"

  case "$name" in
    *.img.zst) format="img.zst" ;;
    *.iso)     format="iso" ;;
    *.vmdk)    format="vmdk" ;;
    *.qcow2)   format="qcow2" ;;
    *)         format="${name##*.}" ;;
  esac

  echo "Publishing $name (size=$size sha256=$sha)"
  aws s3 cp "$path" "s3://${S3_BUCKET}/${S3_PREFIX}/${VERSION}/${name}" --no-progress

  jq --arg name "$name" \
     --argjson size "$size" \
     --arg sha256 "$sha" \
     --arg variant "$VARIANT" \
     --arg arch "$ARCH" \
     --arg suite "$SUITE" \
     --arg format "$format" \
     '. += [{ name: $name, size: $size, sha256: $sha256, variant: $variant, arch: $arch, suite: $suite, format: $format } | del(.[] | select(. == ""))]' \
     "$FRAGMENT" > "$FRAGMENT.tmp"
  mv "$FRAGMENT.tmp" "$FRAGMENT"
done

echo "--- $FRAGMENT ---"
cat "$FRAGMENT"
