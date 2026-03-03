#!/bin/bash
# r[image.snapper.root] r[image.snapper.postgres] r[image.snapper.timers]
#
# Configure Snapper for BTRFS snapshot management.
# This runs inside the chroot during image build.
set -euxo pipefail

# r[image.snapper.root]: Configure snapper for the root subvolume
snapper -c root create-config /
snapper -c root set-config \
    TIMELINE_CREATE=yes \
    TIMELINE_CLEANUP=yes \
    NUMBER_CLEANUP=yes \
    TIMELINE_LIMIT_HOURLY=10 \
    TIMELINE_LIMIT_DAILY=7 \
    TIMELINE_LIMIT_WEEKLY=4 \
    TIMELINE_LIMIT_MONTHLY=12

# r[image.snapper.postgres]: Configure snapper for the PostgreSQL subvolume
mkdir -p /var/lib/postgresql
snapper -c postgres create-config /var/lib/postgresql
snapper -c postgres set-config \
    TIMELINE_CREATE=yes \
    TIMELINE_CLEANUP=yes \
    NUMBER_CLEANUP=yes \
    TIMELINE_LIMIT_HOURLY=10 \
    TIMELINE_LIMIT_DAILY=7 \
    TIMELINE_LIMIT_WEEKLY=4 \
    TIMELINE_LIMIT_MONTHLY=12

# r[image.snapper.timers]: Enable snapper timers
systemctl enable snapper-timeline.timer
systemctl enable snapper-cleanup.timer
