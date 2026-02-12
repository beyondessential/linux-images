#!/bin/bash
set -euxo pipefail

: Create snapper config for root subvolume
snapper -c root create-config /
snapper -c root set-config \
  TIMELINE_CREATE=yes \
  TIMELINE_CLEANUP=yes \
  NUMBER_CLEANUP=yes \
  TIMELINE_LIMIT_HOURLY=10 \
  TIMELINE_LIMIT_DAILY=7 \
  TIMELINE_LIMIT_WEEKLY=4 \
  TIMELINE_LIMIT_MONTHLY=12

: Create snapper config for postgres subvolume
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

: Enable snapper timers
systemctl enable snapper-timeline.timer
systemctl enable snapper-cleanup.timer
