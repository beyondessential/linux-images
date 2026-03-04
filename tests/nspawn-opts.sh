#!/bin/bash
# Shared systemd-nspawn options for container tests.
# Sourced by test-container-install.sh and test-container-isolation.sh
# to keep the container configuration in a single place.

NSPAWN_COMMON_OPTS=(
    --register=no
    --quiet
    --pipe
    --private-network
    --capability=CAP_SYS_ADMIN
    --system-call-filter=mount
    --property=DeviceAllow='block-loop rwm'
    --property=DeviceAllow='block-blkext rwm'
    --property=DeviceAllow='char-misc rwm'
    --property=DeviceAllow='block-device-mapper rwm'
)
