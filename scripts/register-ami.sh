#!/bin/bash
set -euo pipefail

# Register an imported snapshot as an AMI
# Usage: ./register-ami.sh <import-task-id> [region] [arch]

IMPORT_TASK_ID="${1:-}"
REGION="${2:-ap-southeast-2}"
ARCH="${3:-amd64}"

if [ -z "$IMPORT_TASK_ID" ]; then
    echo "Usage: $0 <import-task-id> [region] [arch]"
    echo ""
    echo "Example:"
    echo "  $0 import-snap-1234567890abcdef0 ap-southeast-2 amd64"
    exit 1
fi

if [ "$ARCH" != "amd64" ] && [ "$ARCH" != "arm64" ]; then
    echo "ERROR: Architecture must be 'amd64' or 'arm64'"
    exit 1
fi

echo "=== Registering imported snapshot as AMI ==="
echo "Import Task ID: $IMPORT_TASK_ID"
echo "Region: $REGION"
echo "Architecture: $ARCH"
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
AMI_NAME="ubuntu-24.04-bes-${ARCH}-${TIMESTAMP}"

# Set architecture for AWS
AWS_ARCH="${ARCH/amd64/x86_64}"

echo ""
echo "Registering AMI..."
echo "Name: $AMI_NAME"

AMI_ID=$(aws ec2 register-image \
    --region "$REGION" \
    --name "$AMI_NAME" \
    --description "BES Ubuntu 24.04 ${ARCH} with BTRFS+LUKS encryption" \
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

# Tag the AMI
echo "Tagging AMI..."
aws ec2 create-tags \
    --region "$REGION" \
    --resources "$AMI_ID" "$SNAPSHOT_ID" \
    --tags \
        "Key=Name,Value=${AMI_NAME}" \
        "Key=Os,Value=Ubuntu" \
        "Key=Version,Value=24.04" \
        "Key=Architecture,Value=${ARCH}" \
        "Key=BuildTime,Value=${TIMESTAMP}" \
        "Key=Features,Value=BTRFS+LUKS" \
        "Key=Builder,Value=BES"

echo "Tagging complete"
echo ""
echo "AMI is ready to use:"
echo "  aws ec2 describe-images --region $REGION --image-ids $AMI_ID"
echo ""
echo "Launch an instance:"
echo "  aws ec2 run-instances --region $REGION --image-id $AMI_ID --instance-type t3.small --key-name <your-key>"
