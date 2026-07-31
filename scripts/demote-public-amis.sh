#!/bin/bash
set -euo pipefail

# r[impl image.output.aws-ami-public+4]
# Make every self-owned public AMI in a region private, except those belonging
# to the release we are about to publish. AWS caps public images per region
# (default 5); without this, each release's new public AMIs stack on top of the
# previous release's and eventually exceed the quota at publish time.
#
# Keeping the version we're publishing public makes this safe to run
# concurrently across the per-(arch, suite) matrix legs that all share one
# version: no leg ever demotes an AMI another leg is about to publish, and
# revoking an already-revoked permission is a no-op.
#
# Usage: ./demote-public-amis.sh <region> <keep-version>
#
# Arguments:
#   region        AWS region to sweep
#   keep-version  Release version (Version tag value) to leave public

REGION="${1:-}"
KEEP_VERSION="${2:-}"

if [ -z "$REGION" ] || [ -z "$KEEP_VERSION" ]; then
    echo "Usage: $0 <region> <keep-version>"
    exit 1
fi

echo "Demoting stale public AMIs in $REGION (keeping version $KEEP_VERSION) ..."

# Self-owned public AMIs whose Version tag is missing or differs from the
# version being published, paired with their backing snapshot. A missing tag
# still counts as stale: this is a dedicated publishing account, so anything
# public that isn't the current release is fair game.
list_stale() {
    aws ec2 describe-images \
        --region "$REGION" \
        --owners self \
        --filters "Name=is-public,Values=true" \
        --output json \
    | jq -r --arg keep "$KEEP_VERSION" '
        .Images[]
        | { id: .ImageId,
            snap: (.BlockDeviceMappings[0].Ebs.SnapshotId // ""),
            ver: ((.Tags // []) | map(select(.Key == "Version")) | .[0].Value // "") }
        | select(.ver != $keep)
        | "\(.id)\t\(.snap)"'
}

# Revoke public permissions, then re-check. describe-images is eventually
# consistent, so a single pass can still report a just-revoked AMI as public;
# loop until the region is clean (or give up). The revoke itself is idempotent,
# so re-revoking across rounds is harmless.
MAX_ROUNDS=6
round=0
while true; do
    STALE=$(list_stale)
    if [ -z "$STALE" ]; then
        echo "No stale public AMIs remain in $REGION."
        break
    fi

    round=$((round + 1))
    if [ "$round" -gt "$MAX_ROUNDS" ]; then
        echo "ERROR: public AMIs from other releases still present in $REGION after $MAX_ROUNDS rounds:" >&2
        echo "$STALE" >&2
        exit 1
    fi

    while IFS=$'\t' read -r AMI_ID SNAPSHOT_ID; do
        [ -z "$AMI_ID" ] && continue
        echo "  Making $AMI_ID private (snapshot ${SNAPSHOT_ID:-none}) ..."
        aws ec2 modify-image-attribute \
            --region "$REGION" \
            --image-id "$AMI_ID" \
            --launch-permission 'Remove=[{Group=all}]'
        if [ -n "$SNAPSHOT_ID" ] && [ "$SNAPSHOT_ID" != "None" ]; then
            aws ec2 modify-snapshot-attribute \
                --region "$REGION" \
                --snapshot-id "$SNAPSHOT_ID" \
                --create-volume-permission 'Remove=[{Group=all}]'
        fi
    done <<< "$STALE"

    # Give the revoke a moment to propagate before re-checking.
    sleep 5
done

# r[verify image.output.aws-ami-public+4]
# The loop only exits once list_stale returns empty, so reaching here means no
# AMI from another release is left public — the publish that follows starts
# well under the per-region cap.
echo "Demotion complete."
