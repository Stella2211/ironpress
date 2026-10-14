#!/usr/bin/env bash
set -euo pipefail

# ─── Build native libraries for ironpress ─────────────────────
#
# Usage:
#   ./scripts/build.sh android     # Build all Android ABIs
#   ./scripts/build.sh windows     # Build Windows DLL
#   ./scripts/build.sh apple       # Build iOS + macOS dynamic xcframework
#   ./scripts/build.sh all         # Build everything
#
# Prerequisites:
#   - Rust toolchain (rustup)
#   - cargo-ndk (cargo install cargo-ndk)
#   - Android NDK (via Android Studio or sdkmanager)
#   - cargo-ndk for Android builds
#   - Xcode command line tools for Apple builds (lipo, xcodebuild, codesign)

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

# ─── Apple (iOS + macOS) ──────────────────────────────────────
#
# Produces darwin/ironpress/Frameworks/ironpress_native.xcframework, a set of
# *dynamic* frameworks consumed by both Swift Package Manager
# (`binaryTarget(path:)`) and CocoaPods (`vendored_frameworks`). The Dart side
# opens `ironpress_native.framework/ironpress_native` at runtime.
#
# The framework is intentionally not named `ironpress` so it cannot collide
# with the `ironpress` Swift module/framework produced by the plugin target
# itself (CocoaPods `use_frameworks!` / SwiftPM).

APPLE_FRAMEWORK_NAME="ironpress_native"
APPLE_BUNDLE_ID="dev.rickyirfandi.ironpress.native"
# Keep in sync with darwin/ironpress.podspec and darwin/ironpress/Package.swift.
# These match the minimum deployment targets of Flutter itself.
APPLE_IOS_MIN="15.0"
APPLE_MACOS_MIN="12.0"

apple_cargo() {
    # Prefer the rustup-managed toolchain when available so the configured
    # targets are used even if another rustc is first on PATH.
    if command -v rustup >/dev/null 2>&1; then
        local cargo_bin rustc_bin
        cargo_bin="$(rustup which --toolchain stable cargo)"
        rustc_bin="$(rustup which --toolchain stable rustc)"
        (cd "$RUST_DIR" && RUSTC="$rustc_bin" "$cargo_bin" "$@")
    else
        (cd "$RUST_DIR" && cargo "$@")
    fi
}

apple_crate_version() {
    sed -n 's/^version *= *"\(.*\)"/\1/p' "$RUST_DIR/Cargo.toml" | head -1
}

# write_framework_plist <out> <supported_platform> <min_os_key> <min_os>
write_framework_plist() {
    local out="$1" supported="$2" min_key="$3" min_os="$4"
    local version
    version="$(apple_crate_version)"
    cat > "$out" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>${APPLE_FRAMEWORK_NAME}</string>
	<key>CFBundleIdentifier</key>
	<string>${APPLE_BUNDLE_ID}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>${APPLE_FRAMEWORK_NAME}</string>
	<key>CFBundlePackageType</key>
	<string>FMWK</string>
	<key>CFBundleShortVersionString</key>
	<string>${version}</string>
	<key>CFBundleSignature</key>
	<string>????</string>
	<key>CFBundleSupportedPlatforms</key>
	<array>
		<string>${supported}</string>
	</array>
	<key>CFBundleVersion</key>
	<string>${version}</string>
	<key>${min_key}</key>
	<string>${min_os}</string>
</dict>
</plist>
PLIST
    plutil -lint "$out" >/dev/null
}

# make_ios_framework <dylib> <out_dir> <supported_platform>
make_ios_framework() {
    local dylib="$1" out_dir="$2" supported="$3"
    local fw="$out_dir/$APPLE_FRAMEWORK_NAME.framework"
    rm -rf "$fw"
    mkdir -p "$fw"
    cp "$dylib" "$fw/$APPLE_FRAMEWORK_NAME"
    # Strip local symbols (exported FFI entry points are kept). Cargo's own
    # `strip = true` can silently fail when rust-objcopy is unavailable.
    xcrun strip -x "$fw/$APPLE_FRAMEWORK_NAME"
    install_name_tool -id "@rpath/$APPLE_FRAMEWORK_NAME.framework/$APPLE_FRAMEWORK_NAME" \
        "$fw/$APPLE_FRAMEWORK_NAME"
    write_framework_plist "$fw/Info.plist" "$supported" MinimumOSVersion "$APPLE_IOS_MIN"
    cp "$PROJECT_DIR/darwin/ironpress/Sources/ironpress/PrivacyInfo.xcprivacy" "$fw/"
    codesign --force --sign - --timestamp=none "$fw"
}

# make_macos_framework <dylib> <out_dir>
make_macos_framework() {
    local dylib="$1" out_dir="$2"
    local fw="$out_dir/$APPLE_FRAMEWORK_NAME.framework"
    rm -rf "$fw"
    mkdir -p "$fw/Versions/A/Resources"
    cp "$dylib" "$fw/Versions/A/$APPLE_FRAMEWORK_NAME"
    xcrun strip -x "$fw/Versions/A/$APPLE_FRAMEWORK_NAME"
    install_name_tool -id "@rpath/$APPLE_FRAMEWORK_NAME.framework/Versions/A/$APPLE_FRAMEWORK_NAME" \
        "$fw/Versions/A/$APPLE_FRAMEWORK_NAME"
    write_framework_plist "$fw/Versions/A/Resources/Info.plist" MacOSX LSMinimumSystemVersion "$APPLE_MACOS_MIN"
    cp "$PROJECT_DIR/darwin/ironpress/Sources/ironpress/PrivacyInfo.xcprivacy" "$fw/Versions/A/Resources/"
    ln -s A "$fw/Versions/Current"
    ln -s "Versions/Current/$APPLE_FRAMEWORK_NAME" "$fw/$APPLE_FRAMEWORK_NAME"
    ln -s Versions/Current/Resources "$fw/Resources"
    codesign --force --sign - --timestamp=none "$fw/Versions/A"
}

build_apple() {
    log "Building Apple (iOS + macOS) targets..."

    command -v xcodebuild >/dev/null 2>&1 || err "xcodebuild not found. Install Xcode."

    local targets=(
        aarch64-apple-ios
        aarch64-apple-ios-sim
        x86_64-apple-ios
        aarch64-apple-darwin
        x86_64-apple-darwin
    )
    if command -v rustup >/dev/null 2>&1; then
        rustup target add --toolchain stable "${targets[@]}" >/dev/null 2>&1 || true
    fi

    for target in "${targets[@]}"; do
        log "  → $target"
        IPHONEOS_DEPLOYMENT_TARGET="$APPLE_IOS_MIN" \
        MACOSX_DEPLOYMENT_TARGET="$APPLE_MACOS_MIN" \
            apple_cargo build --release --lib --target "$target"
    done

    local rel="$RUST_DIR/target"
    local stage="$RUST_DIR/target/apple-frameworks"
    rm -rf "$stage"
    mkdir -p "$stage/ios" "$stage/ios-sim" "$stage/macos"

    lipo -create \
        "$rel/aarch64-apple-ios-sim/release/libironpress.dylib" \
        "$rel/x86_64-apple-ios/release/libironpress.dylib" \
        -output "$stage/ios-sim/libironpress.dylib"
    lipo -create \
        "$rel/aarch64-apple-darwin/release/libironpress.dylib" \
        "$rel/x86_64-apple-darwin/release/libironpress.dylib" \
        -output "$stage/macos/libironpress.dylib"

    make_ios_framework "$rel/aarch64-apple-ios/release/libironpress.dylib" "$stage/ios" iPhoneOS
    make_ios_framework "$stage/ios-sim/libironpress.dylib" "$stage/ios-sim" iPhoneSimulator
    make_macos_framework "$stage/macos/libironpress.dylib" "$stage/macos"

    local out="$PROJECT_DIR/darwin/ironpress/Frameworks/$APPLE_FRAMEWORK_NAME.xcframework"
    rm -rf "$out"
    mkdir -p "$(dirname "$out")"
    xcodebuild -create-xcframework \
        -framework "$stage/ios/$APPLE_FRAMEWORK_NAME.framework" \
        -framework "$stage/ios-sim/$APPLE_FRAMEWORK_NAME.framework" \
        -framework "$stage/macos/$APPLE_FRAMEWORK_NAME.framework" \
        -output "$out" >/dev/null

    find "$out" -name "$APPLE_FRAMEWORK_NAME" -type f | while read -r bin; do
        log "  ✓ ${bin#"$PROJECT_DIR"/}: $(lipo -archs "$bin") ($(du -h "$bin" | cut -f1))"
        log "    id: $(otool -D "$bin" | tail -1)"
    done
    log "Apple build complete!"
}

show_usage() {
    echo "Usage: $0 {android|windows|apple|all}"
    echo ""
    echo "Commands:"
    echo "  android    Build Android .so for arm64, armv7, x86_64, x86"
    echo "  windows    Build Windows x64 DLL"
    echo "  apple      Build iOS + macOS dynamic xcframework (macOS host only)"
    echo "  all        Build all platforms"
}

case "${1:-}" in
    android)
        build_android
        ;;
    windows)
        build_windows
        ;;
    apple)
        build_apple
        ;;
    all)
        build_android
        build_windows
        if [[ "$(uname -s)" == "Darwin" ]]; then
            build_apple
        else
            warn "Skipping Apple build (requires a macOS host)."
        fi
        ;;
    *)
        show_usage
        exit 1
        ;;
esac
