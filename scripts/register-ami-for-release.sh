#!/bin/bash
set -euo pipefail

# Register the cloud variant image as an AWS AMI.
# Designed for use in CI (GitHub Actions) after a tagged release.
#
# The cloud image (no LUKS) is the correct variant for AWS — EBS provides
# encryption at rest.
#
# Usage: ./register-ami-for-release.sh <arch> <version> [region] [s3-bucket]
#
# Arguments:
#   arch       Architecture: amd64 or arm64
#   version    Release version string (e.g. "1.2.3", without leading "v")
#   region     AWS region (default: ap-southeast-2)
#   s3-bucket  S3 bucket for import staging (default: bes-ops-tools)
#
# The raw.zst image is expected under output/<arch>/cloud/*.raw.zst

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-}"
VERSION="${2:-}"
REGION="${3:-ap-southeast-2}"
BUCKET="${4:-bes-ops-tools}"

if [ -z "$ARCH" ] || [ -z "$VERSION" ]; then
    echo "Usage: $0 <arch> <version> [region] [s3-bucket]"
    exit 1
fi

case "$ARCH" in
    amd64|arm64) ;;
    *) echo "ERROR: arch must be amd64 or arm64, got: $ARCH"; exit 1 ;;
esac

# r[impl ci.release.aws-ami.name]
AMI_NAME="ubuntu-24.04-bes-cloud-${ARCH}-${VERSION}"
OUTPUT_DIR="$REPO_ROOT/output/${ARCH}/cloud"

echo "Architecture : $ARCH"
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

ZST_FILE=$(find "$OUTPUT_DIR" -maxdepth 1 -name '*.raw.zst' | head -1)
if [ -z "$ZST_FILE" ]; then
    echo "ERROR: No .raw.zst file found in $OUTPUT_DIR"
    exit 1
fi

echo "Decompressing: $ZST_FILE"
RAW_FILE="${ZST_FILE%.zst}"
zstd -d "$ZST_FILE" -o "$RAW_FILE" --force
echo "Decompressed to: $RAW_FILE"
echo ""

# --- Upload to S3 ---

S3_KEY="linux-images/${VERSION}/${AMI_NAME}.raw"
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

# r[impl ci.release.aws-ami]
AWS_ARCH="${ARCH/amd64/x86_64}"
echo "Registering AMI: $AMI_NAME ..."
AMI_ID=$(aws ec2 register-image \
    --region "$REGION" \
    --name "$AMI_NAME" \
    --description "BES Ubuntu 24.04 cloud ${ARCH} ${VERSION} with BTRFS" \
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

# r[impl ci.release.aws-ami.tags]
aws ec2 create-tags \
    --region "$REGION" \
    --resources "$AMI_ID" "$SNAPSHOT_ID" \
    --tags \
        "Key=Name,Value=${AMI_NAME}" \
        "Key=Os,Value=Ubuntu" \
        "Key=OsVersion,Value=24.04" \
        "Key=Variant,Value=cloud" \
        "Key=Architecture,Value=${ARCH}" \
        "Key=Version,Value=${VERSION}" \
        "Key=Features,Value=BTRFS" \
        "Key=Builder,Value=BES"
echo "Tagged AMI and snapshot"
echo ""

echo "Done."
echo "AMI $AMI_ID ($AMI_NAME) is registered and ready."
