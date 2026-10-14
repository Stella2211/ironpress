// ironpress is an FFI-only plugin: all work happens in Dart via `dart:ffi`
// calling into the prebuilt `ironpress_native` framework, which is linked
// and embedded through this package (Swift Package Manager) or the podspec
// (CocoaPods). No platform channel or plugin registration is needed, so this
// target only exists to vend the binary framework and privacy manifest.
enum IronpressNativeLibrary {
  /// Name of the embedded framework opened by the Dart loader.
  static let frameworkName = "ironpress_native"
}
