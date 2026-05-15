#!/bin/bash
set -euo pipefail

# Register the cloud variant image as an AWS AMI.
# Designed for use in CI (GitHub Actions) after a tagged release.
#
# The cloud image (no LUKS) is the correct variant for AWS — EBS provides
# encryption at rest via the snapshot import below.
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
# Required environment:
#   AWS_AMI_KMS_KEY_ID    ARN (or alias) of a customer-managed KMS key whose
#                         key policy grants the BES AWS Organization
#                         permission to launch from snapshots it encrypts.
#                         AWS-managed CMKs (the default aws/ebs key) cannot
#                         be shared cross-account, so without a customer-
#                         managed key the resulting AMI cannot be shared
#                         org-wide.
#   AWS_AMI_SHARE_ORG_ARN ARN of the AWS Organization to grant launch
#                         permission on the AMI and create-volume permission
#                         on its backing snapshot. Format:
#                           arn:aws:organizations::<mgmt-account>:organization/o-XXXXXXXXXX
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

KMS_KEY_ID="${AWS_AMI_KMS_KEY_ID:-}"
if [ -z "$KMS_KEY_ID" ]; then
    echo "ERROR: AWS_AMI_KMS_KEY_ID env var is required."
    echo "       It must be the ARN (or alias) of a customer-managed KMS key"
    echo "       whose policy grants the BES AWS Organization permission to"
    echo "       launch from snapshots it encrypts. Without one the snapshot"
    echo "       defaults to the aws/ebs managed key, which cannot be shared"
    echo "       cross-account."
    exit 1
fi

SHARE_ORG_ARN="${AWS_AMI_SHARE_ORG_ARN:-}"
if [ -z "$SHARE_ORG_ARN" ]; then
    echo "ERROR: AWS_AMI_SHARE_ORG_ARN env var is required."
    echo "       It must be the ARN of the AWS Organization that should be"
    echo "       granted launch permission on the registered AMI (and its"
    echo "       backing snapshot). Format:"
    echo "         arn:aws:organizations::<management-account-id>:organization/o-XXXXXXXXXX"
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
# --encrypted + --kms-key-id force the import to use our customer-managed
# CMK instead of the account's default (aws/ebs), so the resulting snapshot
# and AMI can be shared across the BES AWS Organization.
TASK_ID=$(aws ec2 import-snapshot \
    --region "$REGION" \
    --description "$AMI_NAME" \
    --encrypted \
    --kms-key-id "$KMS_KEY_ID" \
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

# Fail fast if the import landed under a different key (e.g. account-level
# default-encryption pinned to aws/ebs would override an unrecognised alias).
SNAPSHOT_KEY=$(aws ec2 describe-snapshots \
    --region "$REGION" \
    --snapshot-ids "$SNAPSHOT_ID" \
    --query 'Snapshots[0].KmsKeyId' \
    --output text)
case "$SNAPSHOT_KEY" in
    *":alias/aws/ebs"|"")
        echo "ERROR: snapshot $SNAPSHOT_ID is encrypted with $SNAPSHOT_KEY,"
        echo "       not the customer-managed key requested."
        exit 1
        ;;
esac
echo "Snapshot key: $SNAPSHOT_KEY"
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

# --- Grant org-wide launch permission ---

# Grant the org launch permission on the AMI and create-volume permission
# on the snapshot, so other accounts in the org can both run instances from
# the AMI and (if they want to) create volumes directly from the snapshot.
# The customer-managed KMS key set earlier already permits org-wide use.
echo "Granting org launch permission to $SHARE_ORG_ARN ..."
aws ec2 modify-image-attribute \
    --region "$REGION" \
    --image-id "$AMI_ID" \
    --launch-permission "Add=[{OrganizationArn=${SHARE_ORG_ARN}}]"
aws ec2 modify-snapshot-attribute \
    --region "$REGION" \
    --snapshot-id "$SNAPSHOT_ID" \
    --create-volume-permission "Add=[{OrganizationArn=${SHARE_ORG_ARN}}]"

LAUNCH_GRANT=$(aws ec2 describe-image-attribute \
    --region "$REGION" \
    --image-id "$AMI_ID" \
    --attribute launchPermission \
    --query "LaunchPermissions[?OrganizationArn=='${SHARE_ORG_ARN}'] | [0].OrganizationArn" \
    --output text)
if [ "$LAUNCH_GRANT" != "$SHARE_ORG_ARN" ]; then
    echo "ERROR: launch permission for $SHARE_ORG_ARN did not stick on $AMI_ID"
    exit 1
fi
echo "Granted and verified"
echo ""

echo "Done."
echo "AMI $AMI_ID ($AMI_NAME) is registered and shared with $SHARE_ORG_ARN."
