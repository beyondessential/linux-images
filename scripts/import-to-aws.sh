#!/bin/bash
set -euo pipefail

# Import a cloud variant disk image to AWS as an EBS snapshot.
#
# The cloud image (no LUKS) is the correct variant for AWS — encryption at
# rest is provided by EBS.
#
# Usage: ./import-to-aws.sh [amd64|arm64] [region] [s3-bucket]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-amd64}"
REGION="${2:-ap-southeast-2}"
BUCKET_NAME="${3:-bes-image-imports}"

if [ "$ARCH" != "amd64" ] && [ "$ARCH" != "arm64" ]; then
    echo "Usage: $0 [amd64|arm64] [region] [s3-bucket]"
    exit 1
fi

echo "=== Importing cloud image to AWS ==="
echo "Architecture: $ARCH"
echo "Region: $REGION"
echo "S3 Bucket: $BUCKET_NAME"

# Find the cloud image under output/<arch>/cloud/
OUTPUT_DIR="$REPO_ROOT/output/${ARCH}/cloud"
if [ ! -d "$OUTPUT_DIR" ]; then
    echo "ERROR: No cloud images found in $OUTPUT_DIR"
    echo "Build a cloud image first with: just arch=${ARCH} variant=cloud build"
    exit 1
fi

IMAGE_FILE=$(find "$OUTPUT_DIR" -maxdepth 1 -name '*.raw' -not -name '*.raw.zst' -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
if [ -z "$IMAGE_FILE" ]; then
    ZST_FILE=$(find "$OUTPUT_DIR" -maxdepth 1 -name '*.raw.zst' -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
    if [ -z "$ZST_FILE" ]; then
        echo "ERROR: No raw image files found in $OUTPUT_DIR"
        exit 1
    fi
    echo "Found compressed image: $ZST_FILE"
    echo "Decompressing..."
    IMAGE_FILE="${ZST_FILE%.zst}"
    zstd -d "$ZST_FILE" -o "$IMAGE_FILE"
else
    echo "Found image: $IMAGE_FILE"
fi

IMAGE_BASENAME=$(basename "$IMAGE_FILE")
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
AMI_NAME="ubuntu-24.04-bes-cloud-${ARCH}-${TIMESTAMP}"

# Create S3 bucket if it doesn't exist
echo "=== Ensuring S3 bucket exists ==="
if ! aws s3 ls "s3://${BUCKET_NAME}" 2>/dev/null; then
    echo "Creating S3 bucket: $BUCKET_NAME"
    if [ "$REGION" = "us-east-1" ]; then
        aws s3 mb "s3://${BUCKET_NAME}" --region "$REGION"
    else
        aws s3 mb "s3://${BUCKET_NAME}" --region "$REGION" \
            --create-bucket-configuration LocationConstraint="$REGION"
    fi
else
    echo "S3 bucket already exists: $BUCKET_NAME"
fi

# Upload image to S3
S3_KEY="imports/${AMI_NAME}/${IMAGE_BASENAME}"
echo "=== Uploading image to S3 ==="
echo "Destination: s3://${BUCKET_NAME}/${S3_KEY}"
aws s3 cp "$IMAGE_FILE" "s3://${BUCKET_NAME}/${S3_KEY}" \
    --region "$REGION" \
    --storage-class STANDARD

# Create import task
echo "=== Creating import task ==="
IMPORT_TASK_FILE="/tmp/import-task-${TIMESTAMP}.json"

cat > "$IMPORT_TASK_FILE" <<EOF
{
  "Description": "BES Ubuntu 24.04 cloud ${ARCH} with BTRFS",
  "DiskContainers": [
    {
      "Description": "BES Ubuntu 24.04 cloud ${ARCH}",
      "Format": "raw",
      "UserBucket": {
        "S3Bucket": "${BUCKET_NAME}",
        "S3Key": "${S3_KEY}"
      }
    }
  ],
  "RoleName": "vmimport"
}
EOF

echo "Import task configuration:"
cat "$IMPORT_TASK_FILE"

# Check if vmimport role exists
if ! aws iam get-role --role-name vmimport --region "$REGION" 2>/dev/null; then
    echo ""
    echo "ERROR: vmimport IAM role does not exist"
    echo "Create it with the following steps:"
    echo ""
    echo "1. Create trust policy file (trust-policy.json):"
    echo '{'
    echo '  "Version": "2012-10-17",'
    echo '  "Statement": [{'
    echo '    "Effect": "Allow",'
    echo '    "Principal": { "Service": "vmie.amazonaws.com" },'
    echo '    "Action": "sts:AssumeRole",'
    echo '    "Condition": {'
    echo '      "StringEquals": {'
    echo '        "sts:Externalid": "vmimport"'
    echo '      }'
    echo '    }'
    echo '  }]'
    echo '}'
    echo ""
    echo "2. Create role:"
    echo "   aws iam create-role --role-name vmimport --assume-role-policy-document file://trust-policy.json"
    echo ""
    echo "3. Create role policy file (role-policy.json):"
    echo '{'
    echo '  "Version": "2012-10-17",'
    echo '  "Statement": [{'
    echo '    "Effect": "Allow",'
    echo '    "Action": ["s3:GetBucketLocation","s3:GetObject","s3:ListBucket"],'
    echo "    \"Resource\": [\"arn:aws:s3:::${BUCKET_NAME}\",\"arn:aws:s3:::${BUCKET_NAME}/*\"]"
    echo '  },{'
    echo '    "Effect": "Allow",'
    echo '    "Action": ["ec2:ModifySnapshotAttribute","ec2:CopySnapshot","ec2:RegisterImage","ec2:Describe*"],'
    echo '    "Resource": "*"'
    echo '  }]'
    echo '}'
    echo ""
    echo "4. Attach policy:"
    echo "   aws iam put-role-policy --role-name vmimport --policy-name vmimport --policy-document file://role-policy.json"
    echo ""
    rm "$IMPORT_TASK_FILE"
    exit 1
fi

# Start import
echo "=== Starting import ==="
IMPORT_TASK_ID=$(aws ec2 import-snapshot \
    --region "$REGION" \
    --description "BES Ubuntu 24.04 cloud ${ARCH}" \
    --disk-container "file://${IMPORT_TASK_FILE}" \
    --query 'ImportTaskId' \
    --output text)

rm "$IMPORT_TASK_FILE"

echo ""
echo "=== Import task started successfully ==="
echo "Import Task ID: $IMPORT_TASK_ID"
echo "Region: $REGION"
echo "Architecture: $ARCH"
echo ""

# Save import task ID for easy reference
IMPORT_INFO_FILE="${OUTPUT_DIR}/import-${TIMESTAMP}.txt"
cat > "$IMPORT_INFO_FILE" <<EOFINFO
Import Task ID: $IMPORT_TASK_ID
Region: $REGION
Architecture: $ARCH
S3 Bucket: $BUCKET_NAME
S3 Key: $S3_KEY
Started: $(date)
EOFINFO

echo "Import info saved to: $IMPORT_INFO_FILE"
echo ""
echo "Monitor progress with:"
echo "  aws ec2 describe-import-snapshot-tasks --region $REGION --import-task-ids $IMPORT_TASK_ID"
echo ""
echo "Or watch status:"
echo "  watch -n 30 'aws ec2 describe-import-snapshot-tasks --region $REGION --import-task-ids $IMPORT_TASK_ID --query \"ImportSnapshotTasks[0].SnapshotTaskDetail.[Status,Progress,StatusMessage]\" --output table'"
echo ""
echo "Once import is complete (status: completed), register as AMI with:"
echo "  scripts/register-ami.sh $IMPORT_TASK_ID $REGION $ARCH"
echo ""
echo "Then optionally clean up S3:"
echo "  aws s3 rm s3://${BUCKET_NAME}/${S3_KEY} --region $REGION"
echo ""
echo "Note: Import typically takes 20-40 minutes depending on image size"
