#!/bin/bash
set -euo pipefail

# Register the cloud variant image as an AWS AMI and publish it publicly.
# Designed for use in CI (GitHub Actions) after a tagged release.
#
# The cloud image (no LUKS) is the correct variant for AWS. The AMI is made
# public (launch-permission Group=all) so any AWS account can launch from it
# directly — this is an open-source distribution, so there's nothing to gain
# from gating access. Public AMIs require unencrypted snapshots, so the
# publishing account must not have EBS encryption-by-default enabled in the
# target region.
#
# Usage: ./register-ami-for-release.sh <arch> <suite> <version> [region] [s3-bucket]
#
# Arguments:
#   arch       Architecture: amd64 or arm64
#   suite      Ubuntu suite codename: noble or resolute
#   version    Release version string (e.g. "1.2.3", without leading "v")
#   region     AWS region (default: ap-southeast-2)
#   s3-bucket  S3 bucket for import staging (default: bes-ops-tools)
#
# The raw.zst image is expected under output/<arch>/cloud/*.img.zst

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-}"
SUITE="${2:-}"
VERSION="${3:-}"
REGION="${4:-ap-southeast-2}"
BUCKET="${5:-bes-ops-tools}"

if [ -z "$ARCH" ] || [ -z "$SUITE" ] || [ -z "$VERSION" ]; then
    echo "Usage: $0 <arch> <suite> <version> [region] [s3-bucket]"
    exit 1
fi

case "$ARCH" in
    amd64|arm64) ;;
    *) echo "ERROR: arch must be amd64 or arm64, got: $ARCH"; exit 1 ;;
esac

# Map suite codename → numeric Ubuntu version. Keep in lockstep with the
# ubuntu_version mapping in the justfile.
case "$SUITE" in
    noble)    UBUNTU_VERSION="24.04" ;;
    resolute) UBUNTU_VERSION="26.04" ;;
    *) echo "ERROR: unknown suite '$SUITE' (add a mapping here and in the justfile)"; exit 1 ;;
esac

# r[impl image.output.aws-ami]
AMI_NAME="ubuntu-${UBUNTU_VERSION}-bes-cloud-${ARCH}-${VERSION}"
OUTPUT_DIR="$REPO_ROOT/output/${ARCH}/cloud"

echo "Architecture : $ARCH"
echo "Suite        : $SUITE ($UBUNTU_VERSION)"
echo "Version      : $VERSION"
echo "Region       : $REGION"
echo "S3 bucket    : $BUCKET"
echo "AMI name     : $AMI_NAME"
echo ""

# Check whether an AMI with this name already exists (idempotency guard).
EXISTING=$(aws ec2 describe-images \
    --region "$REGION" \
    --owners self \
    --filters "Name=name,Values=${AMI_NAME}" \
    --query 'Images[0].ImageId' \
    --output text)
if [ "$EXISTING" != "None" ] && [ -n "$EXISTING" ]; then
    echo "AMI already exists: $EXISTING — skipping registration."
    exit 0
fi

# --- Find and decompress the image ---

# Match the suite's image specifically. Build artifacts use the
# `ubuntu-<ubuntu_version>-bes-cloud-<arch>-...` pattern (see justfile filestem),
# so anchoring on the version prevents picking up a sibling suite's image if
# both happen to be present.
ZST_FILE=$(find "$OUTPUT_DIR" -maxdepth 1 -name "ubuntu-${UBUNTU_VERSION}-bes-cloud-${ARCH}-*.img.zst" | head -1)
if [ -z "$ZST_FILE" ]; then
    echo "ERROR: No ubuntu-${UBUNTU_VERSION}-bes-cloud-${ARCH}-*.img.zst found in $OUTPUT_DIR"
    exit 1
fi

echo "Decompressing: $ZST_FILE"
RAW_FILE="${ZST_FILE%.zst}"
zstd -d "$ZST_FILE" -o "$RAW_FILE" --force
echo "Decompressed to: $RAW_FILE"
echo ""

# --- Upload to S3 ---

S3_KEY="linux-images/${VERSION}/${AMI_NAME}.img"
echo "Uploading to s3://${BUCKET}/${S3_KEY} ..."
aws s3 cp "$RAW_FILE" "s3://${BUCKET}/${S3_KEY}" \
    --region "$REGION" \
    --no-progress

rm -f "$RAW_FILE"
echo "Upload complete, freed local disk"
echo ""

# --- Start snapshot import ---

echo "Starting snapshot import ..."
TASK_ID=$(aws ec2 import-snapshot \
    --region "$REGION" \
    --description "$AMI_NAME" \
    --disk-container "Description=${AMI_NAME},Format=raw,UserBucket={S3Bucket=${BUCKET},S3Key=${S3_KEY}}" \
    --query 'ImportTaskId' \
    --output text)
echo "Import task: $TASK_ID"
echo ""

# --- Poll until the import completes ---

echo "Waiting for import to complete (typically 20-40 minutes) ..."
MAX_SECS=3600
ELAPSED=0
while true; do
    DETAIL=$(aws ec2 describe-import-snapshot-tasks \
        --region "$REGION" \
        --import-task-ids "$TASK_ID" \
        --query 'ImportSnapshotTasks[0].SnapshotTaskDetail' \
        --output json)
    STATUS=$(echo "$DETAIL" | jq -r '.Status')
    PROGRESS=$(echo "$DETAIL" | jq -r '.Progress // "?"')
    echo "  [${ELAPSED}s] status=$STATUS progress=${PROGRESS}%"
    case "$STATUS" in
        completed)
            echo ""
            break
            ;;
        deleted|deleting)
            echo "ERROR: import task was deleted"
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

# --- Clean up S3 staging object ---

aws s3 rm "s3://${BUCKET}/${S3_KEY}" --region "$REGION"
echo "Cleaned up S3 staging object"

# --- Get snapshot ID ---

SNAPSHOT_ID=$(aws ec2 describe-import-snapshot-tasks \
    --region "$REGION" \
    --import-task-ids "$TASK_ID" \
    --query 'ImportSnapshotTasks[0].SnapshotTaskDetail.SnapshotId' \
    --output text)

if [ -z "$SNAPSHOT_ID" ] || [ "$SNAPSHOT_ID" = "None" ]; then
    echo "ERROR: could not retrieve snapshot ID from completed import task"
    exit 1
fi

echo "Snapshot ID: $SNAPSHOT_ID"
echo ""

# --- Register AMI ---

# r[impl image.output.aws-ami]
AWS_ARCH="${ARCH/amd64/x86_64}"
echo "Registering AMI: $AMI_NAME ..."
AMI_ID=$(aws ec2 register-image \
    --region "$REGION" \
    --name "$AMI_NAME" \
    --description "BES Ubuntu ${UBUNTU_VERSION} cloud ${ARCH} ${VERSION} with BTRFS" \
    --architecture "$AWS_ARCH" \
    --root-device-name /dev/sda1 \
    --block-device-mappings "DeviceName=/dev/sda1,Ebs={SnapshotId=${SNAPSHOT_ID},VolumeType=gp3,DeleteOnTermination=true}" \
    --virtualization-type hvm \
    --ena-support \
    --boot-mode uefi \
    --query 'ImageId' \
    --output text)

if [ -z "$AMI_ID" ]; then
    echo "ERROR: failed to register AMI"
    exit 1
fi

echo "AMI ID: $AMI_ID"
echo ""

# --- Tag AMI and snapshot ---

# r[verify image.output.aws-ami]
aws ec2 create-tags \
    --region "$REGION" \
    --resources "$AMI_ID" "$SNAPSHOT_ID" \
    --tags \
        "Key=Name,Value=${AMI_NAME}" \
        "Key=Os,Value=Ubuntu" \
        "Key=OsVersion,Value=${UBUNTU_VERSION}" \
        "Key=OsCodename,Value=${SUITE}" \
        "Key=Variant,Value=cloud" \
        "Key=Architecture,Value=${ARCH}" \
        "Key=Version,Value=${VERSION}" \
        "Key=Features,Value=BTRFS" \
        "Key=Builder,Value=BES"
echo "Tagged AMI and snapshot"
echo ""

# --- Make AMI and snapshot public ---

# Public launch on the AMI lets any AWS account run instances from it; public
# create-volume on the snapshot lets them attach it as a raw volume too. Both
# only work when the snapshot is unencrypted (enforced upstream by the
# absence of --encrypted on import-snapshot and by EBS encryption-by-default
# being off in this account/region).
echo "Publishing AMI and snapshot ..."
aws ec2 modify-image-attribute \
    --region "$REGION" \
    --image-id "$AMI_ID" \
    --launch-permission 'Add=[{Group=all}]'
aws ec2 modify-snapshot-attribute \
    --region "$REGION" \
    --snapshot-id "$SNAPSHOT_ID" \
    --create-volume-permission 'Add=[{Group=all}]'

PUBLIC=$(aws ec2 describe-images \
    --region "$REGION" \
    --image-ids "$AMI_ID" \
    --query 'Images[0].Public' \
    --output text)
if [ "$PUBLIC" != "True" ]; then
    echo "ERROR: AMI $AMI_ID did not become public (Public=$PUBLIC)"
    exit 1
fi
echo "Published and verified"
echo ""

echo "Done."
echo "AMI $AMI_ID ($AMI_NAME) is registered and public."
