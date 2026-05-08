#!/bin/bash
set -euo pipefail

# Register an imported snapshot as an AMI.
#
# This is the second step after import-to-aws.sh completes. The import
# produces an EBS snapshot; this script registers it as a bootable AMI.
#
# Usage: ./register-ami.sh <import-task-id> <suite> [region] [arch]

IMPORT_TASK_ID="${1:-}"
SUITE="${2:-}"
REGION="${3:-ap-southeast-2}"
ARCH="${4:-amd64}"

if [ -z "$IMPORT_TASK_ID" ] || [ -z "$SUITE" ]; then
    echo "Usage: $0 <import-task-id> <suite> [region] [arch]"
    echo ""
    echo "  suite: noble or resolute"
    echo ""
    echo "Example:"
    echo "  $0 import-snap-1234567890abcdef0 noble ap-southeast-2 amd64"
    exit 1
fi

if [ "$ARCH" != "amd64" ] && [ "$ARCH" != "arm64" ]; then
    echo "ERROR: Architecture must be 'amd64' or 'arm64'"
    exit 1
fi

# Map suite codename → numeric Ubuntu version. Keep in lockstep with the
# ubuntu_version mapping in the justfile.
case "$SUITE" in
    noble)    UBUNTU_VERSION="24.04" ;;
    resolute) UBUNTU_VERSION="26.04" ;;
    *) echo "ERROR: unknown suite '$SUITE' (add a mapping here and in the justfile)"; exit 1 ;;
esac

echo "=== Registering imported snapshot as AMI ==="
echo "Import Task ID: $IMPORT_TASK_ID"
echo "Suite:          $SUITE ($UBUNTU_VERSION)"
echo "Region:         $REGION"
echo "Architecture:   $ARCH"
echo ""

# Check import task status
echo "Checking import task status..."
IMPORT_STATUS=$(aws ec2 describe-import-snapshot-tasks \
    --region "$REGION" \
    --import-task-ids "$IMPORT_TASK_ID" \
    --query 'ImportSnapshotTasks[0].SnapshotTaskDetail.Status' \
    --output text)

if [ "$IMPORT_STATUS" != "completed" ]; then
    echo "ERROR: Import task is not complete. Current status: $IMPORT_STATUS"
    echo ""
    echo "Monitor progress with:"
    echo "  aws ec2 describe-import-snapshot-tasks --region $REGION --import-task-ids $IMPORT_TASK_ID"
    exit 1
fi

echo "Import task completed successfully"

# Get snapshot ID
SNAPSHOT_ID=$(aws ec2 describe-import-snapshot-tasks \
    --region "$REGION" \
    --import-task-ids "$IMPORT_TASK_ID" \
    --query 'ImportSnapshotTasks[0].SnapshotTaskDetail.SnapshotId' \
    --output text)

if [ -z "$SNAPSHOT_ID" ] || [ "$SNAPSHOT_ID" = "None" ]; then
    echo "ERROR: Could not find snapshot ID from import task"
    exit 1
fi

echo "Snapshot ID: $SNAPSHOT_ID"

# Generate AMI name with timestamp
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
AMI_NAME="ubuntu-${UBUNTU_VERSION}-bes-cloud-${ARCH}-${TIMESTAMP}"

# Set architecture for AWS API
AWS_ARCH="${ARCH/amd64/x86_64}"

echo ""
echo "Registering AMI..."
echo "Name: $AMI_NAME"

AMI_ID=$(aws ec2 register-image \
    --region "$REGION" \
    --name "$AMI_NAME" \
    --description "BES Ubuntu ${UBUNTU_VERSION} cloud ${ARCH} with BTRFS" \
    --architecture "$AWS_ARCH" \
    --root-device-name /dev/sda1 \
    --block-device-mappings "DeviceName=/dev/sda1,Ebs={SnapshotId=${SNAPSHOT_ID},VolumeType=gp3,DeleteOnTermination=true}" \
    --virtualization-type hvm \
    --ena-support \
    --boot-mode uefi \
    --query 'ImageId' \
    --output text)

if [ -z "$AMI_ID" ]; then
    echo "ERROR: Failed to register AMI"
    exit 1
fi

echo ""
echo "=== AMI registered successfully ==="
echo "AMI ID: $AMI_ID"
echo "Name: $AMI_NAME"
echo "Region: $REGION"
echo "Architecture: $ARCH"
echo ""

# Tag the AMI and its backing snapshot
echo "Tagging AMI..."
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
        "Key=BuildTime,Value=${TIMESTAMP}" \
        "Key=Features,Value=BTRFS" \
        "Key=Builder,Value=BES"

echo "Tagging complete"
echo ""
echo "AMI is ready to use:"
echo "  aws ec2 describe-images --region $REGION --image-ids $AMI_ID"
echo ""
echo "Launch an instance:"
if [ "$ARCH" = "amd64" ]; then
    echo "  aws ec2 run-instances --region $REGION --image-id $AMI_ID --instance-type t3.small --key-name <your-key>"
else
    echo "  aws ec2 run-instances --region $REGION --image-id $AMI_ID --instance-type t4g.small --key-name <your-key>"
fi
