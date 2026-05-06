#!/bin/bash
# r[image.snapper.root] r[image.snapper.postgres] r[image.snapper.timers]
#
# Configure Snapper for BTRFS snapshot management.
# This runs inside the chroot during image build.
set -euxo pipefail

# r[image.snapper.root]: Configure snapper for the root subvolume
snapper --no-dbus -c root create-config /
snapper --no-dbus -c root set-config \
    TIMELINE_CREATE=yes \
    TIMELINE_CLEANUP=yes \
    NUMBER_CLEANUP=yes \
    NUMBER_LIMIT=10 \
    TIMELINE_LIMIT_HOURLY=6 \
    TIMELINE_LIMIT_DAILY=0 \
    TIMELINE_LIMIT_WEEKLY=0 \
    TIMELINE_LIMIT_MONTHLY=0 \
    TIMELINE_LIMIT_YEARLY=0

# r[image.snapper.postgres]: Configure snapper for the PostgreSQL subvolume
mkdir -p /var/lib/postgresql
snapper --no-dbus -c postgres create-config /var/lib/postgresql
snapper --no-dbus -c postgres set-config \
    TIMELINE_CREATE=yes \
    TIMELINE_CLEANUP=yes \
    NUMBER_CLEANUP=yes \
    NUMBER_LIMIT=10 \
    TIMELINE_LIMIT_HOURLY=6 \
    TIMELINE_LIMIT_DAILY=0 \
    TIMELINE_LIMIT_WEEKLY=0 \
    TIMELINE_LIMIT_MONTHLY=0 \
    TIMELINE_LIMIT_YEARLY=0

# r[image.snapper.timers]: Enable snapper timers
systemctl enable snapper-timeline.timer
systemctl enable snapper-cleanup.timer
