#!/usr/bin/env bash
set -euo pipefail

# ─── Build native libraries for ironpress ─────────────────────
#
# Usage:
#   ./scripts/build.sh android     # Build all Android ABIs
#   ./scripts/build.sh windows     # Build Windows DLL
#   ./scripts/build.sh all         # Build everything
#
# Prerequisites:
#   - Rust toolchain (rustup)
#   - cargo-ndk (cargo install cargo-ndk)
#   - Android NDK (via Android Studio or sdkmanager)
#   - cargo-ndk for Android builds

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RUST_DIR="$PROJECT_DIR/rust"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log() { echo -e "${GREEN}[build]${NC} $1"; }
warn() { echo -e "${YELLOW}[warn]${NC} $1"; }
err() { echo -e "${RED}[error]${NC} $1"; exit 1; }

build_android() {
    log "Building Android targets..."

    command -v cargo-ndk >/dev/null 2>&1 || err "cargo-ndk not found. Install: cargo install cargo-ndk"

    local targets=(
        "aarch64-linux-android:arm64-v8a"
        "armv7-linux-androideabi:armeabi-v7a"
        "x86_64-linux-android:x86_64"
        "i686-linux-android:x86"
    )

    for entry in "${targets[@]}"; do
        IFS=':' read -r target abi <<< "$entry"

        log "  → $target ($abi)"
        rustup target add "$target" 2>/dev/null || true

        (cd "$RUST_DIR" && cargo ndk \
            --target "$target" \
            --platform 21 \
            build --release)

        local out_dir="$PROJECT_DIR/android/src/main/jniLibs/$abi"
        mkdir -p "$out_dir"
        cp "$RUST_DIR/target/$target/release/libironpress.so" "$out_dir/"

        local size=$(du -h "$out_dir/libironpress.so" | cut -f1)
        log "  ✓ $abi: $size"
    done

    log "Android build complete!"
}

build_windows() {
    log "Building Windows DLL..."

    local out_dir="$PROJECT_DIR/windows/libs"
    mkdir -p "$out_dir"

    (cd "$RUST_DIR" && cargo build --release)
    cp "$RUST_DIR/target/release/ironpress.dll" "$out_dir/"

    local size=$(du -h "$out_dir/ironpress.dll" | cut -f1)
    log "  ✓ windows-x64: $size"
    log "Windows build complete!"
}

show_usage() {
    echo "Usage: $0 {android|windows|all}"
    echo ""
    echo "Commands:"
    echo "  android    Build Android .so for arm64, armv7, x86_64, x86"
    echo "  windows    Build Windows x64 DLL"
    echo "  all        Build all platforms"
}

case "${1:-}" in
    android)
        build_android
        ;;
    windows)
        build_windows
        ;;
    all)
        build_android
        build_windows
        ;;
    *)
        show_usage
        exit 1
        ;;
esac
