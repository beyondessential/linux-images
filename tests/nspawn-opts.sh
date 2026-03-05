#!/bin/bash
# Shared systemd-nspawn configuration for all container scripts.
#
# Sourced by:
#   - test-container-install.sh (automated integration tests)
#   - test-container-isolation.sh (isolation smoke test)
#   - try-installer-interactive.sh (interactive trial)
#
# r[impl installer.container.isolation+3]: all container scripts share this
# single file so that the isolation test validates the same nspawn
# configuration that the installer tests and interactive trial use.

# Base nspawn options common to every container invocation.
# Does NOT include --private-network; callers add that separately.
NSPAWN_BASE_OPTS=(
    --register=no
    --quiet
    --pipe
    --capability=CAP_SYS_ADMIN
    --system-call-filter=mount
    --property=DeviceAllow='block-loop rwm'
    --property=DeviceAllow='block-blkext rwm'
    --property=DeviceAllow='char-misc rwm'
    --property=DeviceAllow='block-device-mapper rwm'
)

# Build the full nspawn options array for a given invocation.
#
# Usage:
#   nspawn_opts [--private-network]
#
# Writes result to NSPAWN_OPTS (array).
#
# When --private-network is passed (or when the PRIVATE_NETWORK env var is
# set to "1" / "true" / "yes"), the container is isolated from the host
# network. This is the default for automated tests and serves as the
# enforcement mechanism for r[verify iso.offline].
nspawn_opts() {
    NSPAWN_OPTS=("${NSPAWN_BASE_OPTS[@]}")

    local want_private_network=0
    for arg in "$@"; do
        case "$arg" in
            --private-network) want_private_network=1 ;;
        esac
    done

    # Also accept the decision from an environment variable so that
    # callers can propagate a scenario-level setting without arg plumbing.
    case "${PRIVATE_NETWORK:-}" in
        1|true|yes) want_private_network=1 ;;
    esac

    if [ "$want_private_network" -eq 1 ]; then
        NSPAWN_OPTS+=(--private-network)
    fi
}

# Build the bind-mount array for an installer invocation.
#
# Usage:
#   nspawn_installer_binds <loop-dev> <images-dir> <devices-json> \
#                          [<config-toml>] [<log-bind>]
#
# Writes result to NSPAWN_BINDS (array).
#
# The host /dev is NEVER bind-mounted. The container gets nspawn's own
# private /dev; only the loop device itself is bound in. After partprobe,
# partition device nodes appear on the host's devtmpfs only — the installer
# handles this by reading /sys/class/block/ and creating missing nodes via
# mknod (see r[installer.container.partition-devices+2]).
nspawn_installer_binds() {
    local loop_dev="${1:?nspawn_installer_binds: loop-dev required}"
    local images_dir="${2:?nspawn_installer_binds: images-dir required}"
    local devices_json="${3:?nspawn_installer_binds: devices-json required}"
    local config_toml="${4:-}"
    local log_bind="${5:-}"

    NSPAWN_BINDS=(
        "--bind=$loop_dev"
        "--bind-ro=$images_dir:/run/live/medium/images"
        "--bind-ro=$devices_json:/tmp/devices.json"
    )

    if [ -n "$config_toml" ]; then
        NSPAWN_BINDS+=("--bind-ro=$config_toml:/run/besconf/bes-install.toml")
    fi

    if [ -n "$log_bind" ]; then
        NSPAWN_BINDS+=("--bind=$log_bind")
    fi
}
