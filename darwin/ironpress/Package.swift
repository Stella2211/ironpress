// swift-tools-version: 5.9
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "ironpress",
    platforms: [
        .iOS("15.0"),
        .macOS("12.0"),
    ],
    products: [
        .library(name: "ironpress", targets: ["ironpress"])
    ],
    dependencies: [
        .package(name: "FlutterFramework", path: "../FlutterFramework")
    ],
    targets: [
        .target(
            name: "ironpress",
            dependencies: [
                .product(name: "FlutterFramework", package: "FlutterFramework"),
                "IronpressNative",
            ],
            resources: [
                .process("PrivacyInfo.xcprivacy")
            ]
        ),
        // Prebuilt Rust engine (dynamic framework per platform/slice). Xcode
        // embeds it into the app's Frameworks directory; Dart opens it via
        // `DynamicLibrary.open('ironpress_native.framework/ironpress_native')`.
        // Regenerate with `scripts/build.sh apple`.
        .binaryTarget(
            name: "IronpressNative",
            path: "Frameworks/ironpress_native.xcframework"
        ),
    ]
)
