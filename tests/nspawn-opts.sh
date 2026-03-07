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

# r[impl installer.container.fake-luks]
# Directory containing shim scripts (relative to this file).
_SHIMS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/shims"

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
        "--bind-ro=$images_dir:/run/bes-images"
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
    local pid_file="$state_dir/swtpm.pid"
    local tpm_info
    if ! tpm_info=$(swtpm chardev \
        --tpmstate dir="$state_dir" \
        --tpm2 \
        --vtpm-proxy \
        --daemon \
        --pid file="$pid_file" \
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

    # Read PID from pidfile (most reliable), fall back to fuser, then to
    # scanning /proc for the swtpm process that owns our state directory.
    _SWTPM_PID=""
    local retries=10
    while [ -z "$_SWTPM_PID" ] && [ "$retries" -gt 0 ]; do
        if [ -f "$pid_file" ]; then
            _SWTPM_PID=$(tr -d '[:space:]' < "$pid_file")
        fi
        if [ -z "$_SWTPM_PID" ] && [ -c "$_SWTPM_TPM_DEV" ]; then
            _SWTPM_PID=$(fuser "$_SWTPM_TPM_DEV" 2>/dev/null | tr -d '[:space:]') || true
        fi
        if [ -n "$_SWTPM_PID" ]; then
            break
        fi
        sleep 0.1
        retries=$((retries - 1))
    done

    # Last resort: scan /proc for swtpm with our state directory.
    if [ -z "$_SWTPM_PID" ]; then
        _SWTPM_PID=$(pgrep -f "swtpm.*--tpmstate dir=$state_dir" 2>/dev/null | head -1) || true
    fi

    if [ -z "$_SWTPM_PID" ]; then
        echo "WARNING: could not determine swtpm PID; cleanup may leave an orphan"
    fi

    # Wait briefly for both device nodes to become usable.
    retries=20
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
# Uses multiple strategies to ensure the swtpm process is killed:
#   1. Kill by tracked PID
#   2. Kill by PID file (in case _SWTPM_PID was lost)
#   3. Kill by fuser on the device node
#   4. Kill by pgrep matching the state directory
swtpm_stop() {
    local killed=0

    # Strategy 1: tracked PID
    if [ -n "$_SWTPM_PID" ]; then
        kill "$_SWTPM_PID" 2>/dev/null && killed=1
        wait "$_SWTPM_PID" 2>/dev/null || true
    fi

    # Strategy 2: PID file
    if [ "$killed" -eq 0 ] && [ -n "$_SWTPM_STATE_DIR" ] \
            && [ -f "$_SWTPM_STATE_DIR/swtpm.pid" ]; then
        local file_pid
        file_pid=$(tr -d '[:space:]' < "$_SWTPM_STATE_DIR/swtpm.pid")
        if [ -n "$file_pid" ]; then
            kill "$file_pid" 2>/dev/null && killed=1
            wait "$file_pid" 2>/dev/null || true
        fi
    fi

    # Strategy 3: fuser on the device node
    if [ "$killed" -eq 0 ] && [ -n "$_SWTPM_TPM_DEV" ] \
            && [ -c "$_SWTPM_TPM_DEV" ]; then
        local dev_pid
        dev_pid=$(fuser "$_SWTPM_TPM_DEV" 2>/dev/null | tr -d '[:space:]') || true
        if [ -n "$dev_pid" ]; then
            kill "$dev_pid" 2>/dev/null && killed=1
            wait "$dev_pid" 2>/dev/null || true
        fi
    fi

    # Strategy 4: pgrep for swtpm with our state directory
    if [ "$killed" -eq 0 ] && [ -n "$_SWTPM_STATE_DIR" ]; then
        local pgrep_pid
        pgrep_pid=$(pgrep -f "swtpm.*--tpmstate dir=$_SWTPM_STATE_DIR" 2>/dev/null | head -1) || true
        if [ -n "$pgrep_pid" ]; then
            kill "$pgrep_pid" 2>/dev/null && killed=1
            wait "$pgrep_pid" 2>/dev/null || true
        fi
    fi

    _SWTPM_PID=""
    _SWTPM_TPM_DEV=""
    _SWTPM_TPMRM_DEV=""
    _SWTPM_STATE_DIR=""
}

# ============================================================
# Fake-LUKS helpers for CI environments
# ============================================================

# r[impl installer.container.fake-luks]
# Detect whether real dm-crypt / kernel-keyring LUKS operations are
# available. Sets BES_FAKE_LUKS=1 when they are not.
#
# The caller can force fake mode by setting BES_FAKE_LUKS=1 before calling,
# or force real mode by setting BES_FAKE_LUKS=0.
#
# Usage:
#   luks_detect_or_fake
#
# After this call, BES_FAKE_LUKS is "1" (fake) or "0" (real).
luks_detect_or_fake() {
    # Honour explicit override.
    case "${BES_FAKE_LUKS:-}" in
        0) echo "    luks: real mode (BES_FAKE_LUKS=0 forced)"; return ;;
        1) echo "    luks: fake mode (BES_FAKE_LUKS=1 forced)"; return ;;
    esac

    # Auto-detect: attempt to format+open a tiny LUKS volume on a temp file.
    # If this fails (e.g. "Failed to load key in kernel keyring"), fall back
    # to fake mode.
    local probe_dir
    probe_dir="$(mktemp -d -t bes-luks-probe-XXXXXX)"
    local probe_img="$probe_dir/probe.img"
    local probe_loop="" probe_ok=0

    truncate -s 16M "$probe_img"
    probe_loop="$(losetup --show --find "$probe_img" 2>/dev/null)" || true

    if [ -n "$probe_loop" ]; then
        # Try a quick luksFormat + open + close cycle.
        local probe_kf="$probe_dir/kf"
        printf 'probe-passphrase' > "$probe_kf"
        if cryptsetup luksFormat --type luks2 --batch-mode \
                --pbkdf pbkdf2 --pbkdf-force-iterations 1000 \
                "$probe_loop" --key-file "$probe_kf" 2>/dev/null \
           && cryptsetup open "$probe_loop" bes-luks-probe \
                --key-file "$probe_kf" 2>/dev/null; then
            cryptsetup close bes-luks-probe 2>/dev/null || true
            probe_ok=1
        fi
        losetup -d "$probe_loop" 2>/dev/null || true
    fi

    rm -rf "$probe_dir"

    if [ "$probe_ok" -eq 1 ]; then
        BES_FAKE_LUKS=0
        echo "    luks: real mode (probe succeeded)"
    else
        BES_FAKE_LUKS=1
        echo "    luks: fake mode (probe failed, kernel keyring unavailable)"
    fi
}

# Install the fake cryptsetup and systemd-cryptenroll shims into a container
# rootfs, replacing the real binaries.
#
# Usage:
#   install_fake_luks_shims <rootfs-dir>
#
# Only call when BES_FAKE_LUKS=1.
install_fake_luks_shims() {
    local rootfs="${1:?install_fake_luks_shims: rootfs-dir required}"

    if [ ! -f "$_SHIMS_DIR/cryptsetup" ]; then
        echo "ERROR: shim not found: $_SHIMS_DIR/cryptsetup"
        return 1
    fi

    # Replace the real cryptsetup with the shim.
    # Back up the original in case the rootfs is reused.
    local cs_path="$rootfs/usr/sbin/cryptsetup"
    if [ -f "$cs_path" ] && [ ! -f "$cs_path.real" ]; then
        mv "$cs_path" "$cs_path.real"
    fi
    cp "$_SHIMS_DIR/cryptsetup" "$cs_path"
    chmod +x "$cs_path"

    # Replace systemd-cryptenroll.
    local ce_path="$rootfs/usr/bin/systemd-cryptenroll"
    if [ -f "$ce_path" ] && [ ! -f "$ce_path.real" ]; then
        mv "$ce_path" "$ce_path.real"
    fi
    cp "$_SHIMS_DIR/systemd-cryptenroll" "$ce_path"
    chmod +x "$ce_path"

    echo "    installed fake-LUKS shims into $rootfs"
}

# Open a "LUKS" volume for host-side verification.
#
# In real mode, calls cryptsetup open. In fake mode (BES_FAKE_LUKS=1),
# creates a symlink at /dev/mapper/<name> pointing to the raw partition
# (the partition is unencrypted btrfs in fake mode).
#
# Usage:
#   host_luks_open <device> <name> [--key-file <keyfile>]
#
# Returns 0 on success, non-zero on failure.
host_luks_open() {
    local device="$1" name="$2"
    shift 2

    if [ "${BES_FAKE_LUKS:-0}" = "1" ]; then
        mkdir -p /dev/mapper
        ln -sf "$device" "/dev/mapper/$name"
        return 0
    else
        cryptsetup open "$device" "$name" "$@"
    fi
}

# Close a "LUKS" volume for host-side verification.
#
# In real mode, calls cryptsetup close. In fake mode, removes the symlink.
#
# Usage:
#   host_luks_close <name>
host_luks_close() {
    local name="$1"

    if [ "${BES_FAKE_LUKS:-0}" = "1" ]; then
        rm -f "/dev/mapper/$name"
    else
        cryptsetup close "$name"
    fi
}

# Clean up any fake-LUKS mapper symlinks left behind.
# Safe to call in both real and fake mode.
#
# Usage:
#   host_luks_cleanup <name>
host_luks_cleanup() {
    local name="$1"

    if [ "${BES_FAKE_LUKS:-0}" = "1" ]; then
        rm -f "/dev/mapper/$name" 2>/dev/null || true
    else
        cryptsetup close "$name" 2>/dev/null || true
    fi
}
