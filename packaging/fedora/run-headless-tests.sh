#!/usr/bin/env bash
set -euo pipefail

if [[ $# == 0 ]]; then
    printf 'Usage: %s COMMAND [ARG ...]\n' "$0" >&2
    exit 2
fi

original_home=$HOME
export CARGO_HOME=${CARGO_HOME:-$original_home/.cargo}
export RUSTUP_HOME=${RUSTUP_HOME:-$original_home/.rustup}

sandbox=$(mktemp -d "${TMPDIR:-/tmp}/monogram-headless.XXXXXXXX")
trap 'rm -rf -- "$sandbox"' EXIT
mkdir -p "$sandbox"/{home,runtime,config,cache,data,state}
chmod 0700 "$sandbox/runtime"

export HOME=$sandbox/home
export XDG_RUNTIME_DIR=$sandbox/runtime
export XDG_CONFIG_HOME=$sandbox/config
export XDG_CACHE_HOME=$sandbox/cache
export XDG_DATA_HOME=$sandbox/data
export XDG_STATE_HOME=$sandbox/state
export WAYLAND_DISPLAY=monogram-test-$$
export LIBGL_ALWAYS_SOFTWARE=1
export LP_NUM_THREADS=${LP_NUM_THREADS:-2}
export GSETTINGS_BACKEND=memory
export NO_AT_BRIDGE=1
export GTK_A11Y=none
export GST_REGISTRY=$sandbox/gstreamer-registry.bin
unset DISPLAY

env -u DBUS_SESSION_BUS_ADDRESS dbus-run-session -- bash -c '
    set -euo pipefail
    kwin_wayland --virtual --socket "$WAYLAND_DISPLAY" \
        --width 1280 --height 800 --scale 1 --output-count 1 \
        --no-lockscreen --no-global-shortcuts --no-kactivities \
        >"$XDG_RUNTIME_DIR/kwin.log" 2>&1 &
    kwin_pid=$!
    cleanup() {
        status=$?
        trap - EXIT
        if kill -0 "$kwin_pid" 2>/dev/null; then
            kill -TERM "$kwin_pid"
        fi
        wait "$kwin_pid" 2>/dev/null || true
        if [[ $status != 0 ]]; then
            cat "$XDG_RUNTIME_DIR/kwin.log" >&2
        fi
        exit "$status"
    }
    trap cleanup EXIT

    for _ in {1..100}; do
        if [[ -S $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY ]]; then
            break
        fi
        if ! kill -0 "$kwin_pid" 2>/dev/null; then
            printf "Isolated kwin_wayland exited before creating its socket\n" >&2
            exit 1
        fi
        sleep 0.1
    done
    if [[ ! -S $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY ]]; then
        printf "Timed out waiting for the isolated Wayland socket\n" >&2
        exit 1
    fi

    "$@"
' bash "$@"
