#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/native-smoke.sh [--clean]

Runs the bundled vSomeIP request, response, publish, notification, and
point-to-point native smoke tests serially. Use --clean to rebuild vsomeip-sys.
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
export RUSTFLAGS="${RUSTFLAGS:--Dwarnings}"
toolchain="${RUST_TOOLCHAIN:-1.95.0}"

cargo "+$toolchain" test --locked --all-features -p up-transport-vsomeip \
    --test client_service \
    --test notification \
    --test point_to_point \
    --test publisher_subscriber \
    --test register_unregister \
    -- --test-threads=1
