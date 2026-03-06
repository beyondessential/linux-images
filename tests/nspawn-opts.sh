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

# ============================================================
# State: tracked by this library, cleaned up by swtpm_stop.
# ============================================================
_SWTPM_PID=""
_SWTPM_TPM_DEV=""
_SWTPM_TPMRM_DEV=""
_SWTPM_STATE_DIR=""

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
    --property=DeviceAllow='char-tpm rwm'
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

    # If swtpm_start was called, bind both the raw TPM device and the
    # resource manager device into the container as /dev/tpm0 and
    # /dev/tpmrm0. systemd-cryptenroll --tpm2-device=auto uses the resource
    # manager (/dev/tpmrmN), while the installer's tpm_present check looks
    # for /dev/tpm0. The host devices may be /dev/tpm1 + /dev/tpmrm1 (or
    # higher) when swtpm uses vtpm-proxy.
    if [ -n "$_SWTPM_TPM_DEV" ]; then
        NSPAWN_BINDS+=("--bind=$_SWTPM_TPM_DEV:/dev/tpm0")
    fi
    if [ -n "$_SWTPM_TPMRM_DEV" ]; then
        NSPAWN_BINDS+=("--bind=$_SWTPM_TPMRM_DEV:/dev/tpmrm0")
    fi
}

# ============================================================
# Software TPM helpers
# ============================================================

# r[impl installer.container.swtpm]
# Start a software TPM 2.0 emulator via swtpm chardev + vtpm-proxy.
#
# Usage:
#   swtpm_start <state-dir>
#
# The state-dir is where swtpm persists its TPM state (NV storage etc.).
# On success, sets _SWTPM_PID, _SWTPM_TPM_DEV, and _SWTPM_TPMRM_DEV so
# that nspawn_installer_binds can bind the devices and swtpm_stop can
# clean up.
#
# Prerequisites:
#   - swtpm binary
#   - tpm_vtpm_proxy kernel module loaded
swtpm_start() {
    local state_dir="${1:?swtpm_start: state-dir required}"
    mkdir -p "$state_dir"
    _SWTPM_STATE_DIR="$state_dir"

    if ! command -v swtpm &>/dev/null; then
        echo "ERROR: swtpm not found; install the swtpm package"
        return 1
    fi

    if [ ! -e /dev/vtpmx ]; then
        modprobe tpm_vtpm_proxy 2>/dev/null || true
    fi
    if [ ! -e /dev/vtpmx ]; then
        echo "ERROR: /dev/vtpmx not available; load the tpm_vtpm_proxy kernel module"
        return 1
    fi

    # swtpm chardev --vtpm-proxy creates a /dev/tpmN device via the kernel's
    # vtpm-proxy subsystem. It prints the device path to stdout before
    # daemonizing. The kernel also creates a corresponding /dev/tpmrmN
    # resource manager device.
    local swtpm_log="$state_dir/swtpm.log"
    local tpm_info
    if ! tpm_info=$(swtpm chardev \
        --tpmstate dir="$state_dir" \
        --tpm2 \
        --vtpm-proxy \
        --daemon \
        --log file="$swtpm_log",level=5 \
        2>&1); then
        echo "ERROR: swtpm failed to start: $tpm_info"
        return 1
    fi

    # swtpm prints something like:
    #   New TPM device: /dev/tpm1 (major/minor = 253/1)
    _SWTPM_TPM_DEV=$(echo "$tpm_info" | grep -oP '/dev/tpm\d+')
    if [ -z "$_SWTPM_TPM_DEV" ]; then
        echo "ERROR: could not determine swtpm device path from output: $tpm_info"
        return 1
    fi

    # Derive the resource manager device path: /dev/tpmN -> /dev/tpmrmN
    local tpm_num
    tpm_num=$(echo "$_SWTPM_TPM_DEV" | grep -oP '\d+$')
    _SWTPM_TPMRM_DEV="/dev/tpmrm${tpm_num}"

    # swtpm daemonizes; find its PID via the device it holds open.
    _SWTPM_PID=$(fuser "$_SWTPM_TPM_DEV" 2>/dev/null | tr -d '[:space:]') || true

    # Wait briefly for both device nodes to become usable.
    local retries=20
    while [ "$retries" -gt 0 ]; do
        if [ -c "$_SWTPM_TPM_DEV" ] && [ -c "$_SWTPM_TPMRM_DEV" ]; then
            break
        fi
        sleep 0.1
        retries=$((retries - 1))
    done

    if [ ! -c "$_SWTPM_TPM_DEV" ]; then
        echo "ERROR: swtpm device $_SWTPM_TPM_DEV did not appear"
        swtpm_stop
        return 1
    fi

    if [ ! -c "$_SWTPM_TPMRM_DEV" ]; then
        echo "ERROR: swtpm resource manager device $_SWTPM_TPMRM_DEV did not appear"
        swtpm_stop
        return 1
    fi

    echo "    swtpm: $_SWTPM_TPM_DEV + $_SWTPM_TPMRM_DEV (pid ${_SWTPM_PID:-unknown})"
}

# Stop the software TPM started by swtpm_start.
#
# Safe to call even if swtpm_start was never called (no-op).
swtpm_stop() {
    if [ -n "$_SWTPM_PID" ]; then
        kill "$_SWTPM_PID" 2>/dev/null || true
        wait "$_SWTPM_PID" 2>/dev/null || true
        _SWTPM_PID=""
    fi
    _SWTPM_TPM_DEV=""
    _SWTPM_TPMRM_DEV=""
    _SWTPM_STATE_DIR=""
}
