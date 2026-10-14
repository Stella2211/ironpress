import 'dart:ffi';
import 'dart:io';

const String _libName = 'ironpress';

/// Name of the dynamic framework shipped in
/// `darwin/ironpress/Frameworks/ironpress_native.xcframework` for iOS and
/// macOS. It differs from the plugin's own `ironpress` Swift module so the two
/// never collide when CocoaPods or Swift Package Manager build frameworks.
const String _darwinFrameworkName = 'ironpress_native';

/// Framework-relative path resolved by dyld through the app's `@rpath`
/// (`Frameworks/` on iOS, `Contents/Frameworks/` on macOS).
const String _darwinFrameworkPath =
    '$_darwinFrameworkName.framework/$_darwinFrameworkName';

/// Exported symbol used to verify that [DynamicLibrary.process] actually
/// contains the engine before falling back to it.
const String _probeSymbol = 'ironpress_abi_version';

/// Load the native library for the current platform.
///
/// Throws [UnsupportedError] if the current platform is not supported.
/// Throws [StateError] with actionable message if the library cannot be loaded.
DynamicLibrary loadNativeLibrary() {
  if (Platform.isAndroid) {
    return _tryOpenCandidates(
      const ['libironpress.so'],
      platform: 'Android',
      packagedHint:
          'Ensure the packaged Android `jniLibs` include `libironpress.so`.',
      devHint: 'If you are modifying the native code, rebuild the Android library and package it into the app before running.',
    );
  }

  if (Platform.isIOS) {
    return _tryOpenCandidates(
      const [_darwinFrameworkPath],
      platform: 'iOS',
      packagedHint:
          'In a packaged Flutter iOS app, `$_darwinFrameworkName.framework` should be embedded automatically by Swift Package Manager or CocoaPods.',
      devHint: 'If the app links the engine statically instead, ensure its symbols are exported from the main executable.',
      processFallback: true,
    );
  }

  if (Platform.isMacOS) {
    return _tryOpenCandidates(
      _desktopCandidates(_darwinFrameworkPath, const [
        'darwin',
        'ironpress',
        'Frameworks',
        '$_darwinFrameworkName.xcframework',
        'macos-arm64_x86_64',
        _darwinFrameworkPath,
      ]),
      platform: 'macOS',
      packagedHint:
          'In a packaged Flutter macOS app, `$_darwinFrameworkName.framework` should be embedded automatically by Swift Package Manager or CocoaPods.',
      devHint: 'When running directly from this package checkout, ensure the repo `darwin/ironpress/Frameworks` directory is reachable from the current working directory or `DYLD_FRAMEWORK_PATH`.',
      processFallback: true,
    );
  }

  if (Platform.isLinux) {
    return _tryOpenCandidates(
      _desktopCandidates('lib$_libName.so', const [
        'linux',
        'libs',
        'libironpress.so',
      ]),
      platform: 'Linux',
      packagedHint: 'In a packaged Flutter Linux app, `libironpress.so` should be bundled automatically.',
      devHint: 'When running directly from this package checkout, ensure the repo `linux/libs` directory is reachable from the current working directory or `LD_LIBRARY_PATH`.',
    );
  }

  if (Platform.isWindows) {
    return _tryOpenCandidates(
      _desktopCandidates('$_libName.dll', const [
        'windows',
        'libs',
        '$_libName.dll',
      ]),
      platform: 'Windows',
      packagedHint: 'In a packaged Flutter Windows app, `ironpress.dll` should be bundled automatically.',
      devHint: 'When running directly from this package checkout, ensure the repo `windows/libs` directory is reachable from the current working directory or `PATH`.',
    );
  }

  throw UnsupportedError(
    'ironpress: unsupported platform ${Platform.operatingSystem}',
  );
}

List<String> _desktopCandidates(
  String bundledName,
  List<String> fallbackSegments,
) {
  final candidates = <String>[bundledName];
  var dir = Directory.current.absolute;
  for (var depth = 0; depth < 3; depth++) {
    candidates.add(_joinPath(dir.path, fallbackSegments));
    final parent = dir.parent;
    if (parent.path == dir.path) {
      break;
    }
    dir = parent;
  }
  return candidates.toSet().toList();
}

String _joinPath(String root, List<String> segments) {
  return [root, ...segments].join(Platform.pathSeparator);
}

DynamicLibrary _tryOpenCandidates(
  List<String> candidates, {
  required String platform,
  required String packagedHint,
  required String devHint,
  bool processFallback = false,
}) {
  final errors = <String>[];
  for (final candidate in candidates) {
    try {
      return DynamicLibrary.open(candidate);
    } catch (error) {
      errors.add('  - $candidate: $error');
    }
  }

  if (processFallback) {
    // Legacy integrations (e.g. a statically linked engine) expose the
    // symbols from the host process instead of a separate framework.
    final process = DynamicLibrary.process();
    if (process.providesSymbol(_probeSymbol)) {
      return process;
    }
    errors.add('  - <process>: symbol `$_probeSymbol` not found');
  }

  throw StateError(
    'ironpress: failed to load the native library on $platform.\n'
    '$packagedHint\n'
    '$devHint\n'
    'Attempted locations:\n'
    '${errors.join('\n')}',
  );
}
