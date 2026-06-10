#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/native-smoke.sh [--clean]

Runs the default bundled vSomeIP smoke test. Use --clean when recovering from a
stale or empty VSOMEIP_INSTALL_PATH.
EOF
}

clean=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --clean)
            clean=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

source build/envsetup.sh highest

: "${VSOMEIP_INSTALL_PATH:=$repo_root/vsomeip-install}"
export VSOMEIP_INSTALL_PATH
mkdir -p "$VSOMEIP_INSTALL_PATH"

if [[ "$clean" -eq 1 ]]; then
    cargo clean -p vsomeip-sys
fi

export LD_LIBRARY_PATH="$VSOMEIP_INSTALL_PATH/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cargo test -p vsomeip-sys payload_len --locked
