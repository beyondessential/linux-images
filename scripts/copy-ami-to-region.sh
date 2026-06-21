#!/bin/bash
set -euo pipefail

# Copy a registered AMI from the source region into a target region and
# make the copy public.
#
# AWS AMIs are region-scoped: an AMI registered in ap-southeast-2 can only
# be launched directly from ap-southeast-2. Consumers in other regions
# would otherwise have to copy-image themselves before they could launch.
# This script does that copy on their behalf for the regions we care to
# mirror to (see the copy-amis job matrix in .github/workflows/build.yml).
#
# Like register-ami-for-release.sh, the copy is made public
# (launch-permission Group=all + create-volume-permission Group=all on the
# backing snapshot). Public AMIs require unencrypted snapshots; copy-image
# preserves the source snapshot's encryption status when no key is
# specified, so as long as the source is unencrypted the copy is too.
#
# Usage: ./copy-ami-to-region.sh <arch> <suite> <version> <source-region> <target-region>
#
# Arguments:
#   arch           Architecture: amd64 or arm64
#   suite          Ubuntu suite codename: noble or resolute
#   version        Release version string (e.g. "1.2.3", without leading "v")
#   source-region  AWS region where the AMI was originally registered
#   target-region  AWS region to copy the AMI into

ARCH="${1:-}"
SUITE="${2:-}"
VERSION="${3:-}"
SOURCE_REGION="${4:-}"
TARGET_REGION="${5:-}"

if [ -z "$ARCH" ] || [ -z "$SUITE" ] || [ -z "$VERSION" ] || [ -z "$SOURCE_REGION" ] || [ -z "$TARGET_REGION" ]; then
    echo "Usage: $0 <arch> <suite> <version> <source-region> <target-region>"
    exit 1
fi

case "$ARCH" in
    amd64|arm64) ;;
    *) echo "ERROR: arch must be amd64 or arm64, got: $ARCH"; exit 1 ;;
esac

# Keep in lockstep with register-ami-for-release.sh.
case "$SUITE" in
    noble)    UBUNTU_VERSION="24.04" ;;
    resolute) UBUNTU_VERSION="26.04" ;;
    *) echo "ERROR: unknown suite '$SUITE' (add a mapping here and in the justfile)"; exit 1 ;;
esac

AMI_NAME="ubuntu-${UBUNTU_VERSION}-bes-cloud-${ARCH}-${VERSION}"

echo "Architecture  : $ARCH"
echo "Suite         : $SUITE ($UBUNTU_VERSION)"
echo "Version       : $VERSION"
echo "Source region : $SOURCE_REGION"
echo "Target region : $TARGET_REGION"
echo "AMI name      : $AMI_NAME"
echo ""

# --- Idempotency guard ---

EXISTING=$(aws ec2 describe-images \
    --region "$TARGET_REGION" \
    --owners self \
    --filters "Name=name,Values=${AMI_NAME}" \
    --query 'Images[0].ImageId' \
    --output text)
if [ "$EXISTING" != "None" ] && [ -n "$EXISTING" ]; then
    echo "Copy already exists in $TARGET_REGION: $EXISTING — skipping."
    exit 0
fi

# --- Resolve source AMI ---

SOURCE_AMI=$(aws ec2 describe-images \
    --region "$SOURCE_REGION" \
    --owners self \
    --filters "Name=name,Values=${AMI_NAME}" \
    --query 'Images[0].ImageId' \
    --output text)
if [ "$SOURCE_AMI" = "None" ] || [ -z "$SOURCE_AMI" ]; then
    echo "ERROR: source AMI $AMI_NAME not found in $SOURCE_REGION"
    exit 1
fi
echo "Source AMI: $SOURCE_AMI"
echo ""

# --- Initiate copy ---

# Tag the AMI at creation time. The backing snapshot doesn't get tagged
# from --tag-specifications=image, so we tag it explicitly once the copy
# becomes available and we know the snapshot ID.
TAGS="Key=Name,Value=${AMI_NAME} Key=Os,Value=Ubuntu Key=OsVersion,Value=${UBUNTU_VERSION} Key=OsCodename,Value=${SUITE} Key=Variant,Value=cloud Key=Architecture,Value=${ARCH} Key=Version,Value=${VERSION} Key=Features,Value=BTRFS Key=Builder,Value=BES"
TAG_SPEC="ResourceType=image,Tags=[{Key=Name,Value=${AMI_NAME}},{Key=Os,Value=Ubuntu},{Key=OsVersion,Value=${UBUNTU_VERSION}},{Key=OsCodename,Value=${SUITE}},{Key=Variant,Value=cloud},{Key=Architecture,Value=${ARCH}},{Key=Version,Value=${VERSION}},{Key=Features,Value=BTRFS},{Key=Builder,Value=BES}]"

echo "Initiating copy-image ..."
COPY_AMI=$(aws ec2 copy-image \
    --region "$TARGET_REGION" \
    --source-region "$SOURCE_REGION" \
    --source-image-id "$SOURCE_AMI" \
    --name "$AMI_NAME" \
    --description "BES Ubuntu ${UBUNTU_VERSION} cloud ${ARCH} ${VERSION} with BTRFS" \
    --tag-specifications "$TAG_SPEC" \
    --query 'ImageId' \
    --output text)
echo "Copy AMI ID: $COPY_AMI"
echo ""

# --- Poll until copy completes ---

echo "Waiting for copy to become available (typically 5-20 minutes) ..."
MAX_SECS=3600
ELAPSED=0
while true; do
    STATE=$(aws ec2 describe-images \
        --region "$TARGET_REGION" \
        --image-ids "$COPY_AMI" \
        --query 'Images[0].State' \
        --output text)
    echo "  [${ELAPSED}s] state=$STATE"
    case "$STATE" in
        available) echo ""; break ;;
        failed|invalid|error|deregistered)
            REASON=$(aws ec2 describe-images \
                --region "$TARGET_REGION" \
                --image-ids "$COPY_AMI" \
                --query 'Images[0].StateReason' \
                --output text)
            echo "ERROR: copy entered terminal state $STATE: $REASON"
            exit 1
            ;;
    esac
    if [ "$ELAPSED" -ge "$MAX_SECS" ]; then
        echo "ERROR: timed out after ${MAX_SECS}s"
        exit 1
    fi
    sleep 30
    ELAPSED=$((ELAPSED + 30))
done

# --- Tag the backing snapshot ---

SNAPSHOT_ID=$(aws ec2 describe-images \
    --region "$TARGET_REGION" \
    --image-ids "$COPY_AMI" \
    --query 'Images[0].BlockDeviceMappings[0].Ebs.SnapshotId' \
    --output text)
# shellcheck disable=SC2086 # $TAGS is intentionally word-split into AWS CLI tag args
aws ec2 create-tags \
    --region "$TARGET_REGION" \
    --resources "$SNAPSHOT_ID" \
    --tags $TAGS
echo "Tagged backing snapshot $SNAPSHOT_ID"
echo ""

# --- Make AMI and snapshot public ---

echo "Publishing AMI and snapshot ..."
aws ec2 modify-image-attribute \
    --region "$TARGET_REGION" \
    --image-id "$COPY_AMI" \
    --launch-permission 'Add=[{Group=all}]'
aws ec2 modify-snapshot-attribute \
    --region "$TARGET_REGION" \
    --snapshot-id "$SNAPSHOT_ID" \
    --create-volume-permission 'Add=[{Group=all}]'

PUBLIC=$(aws ec2 describe-images \
    --region "$TARGET_REGION" \
    --image-ids "$COPY_AMI" \
    --query 'Images[0].Public' \
    --output text)
if [ "$PUBLIC" != "True" ]; then
    echo "ERROR: copy $COPY_AMI did not become public (Public=$PUBLIC)"
    exit 1
fi
echo "Published and verified"
echo ""

echo "Done."
echo "AMI $COPY_AMI ($AMI_NAME) is registered and public in $TARGET_REGION."
