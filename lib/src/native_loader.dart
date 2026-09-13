import 'dart:ffi';
import 'dart:io';

const String _libName = 'ironpress';

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
    return DynamicLibrary.process();
  }

  if (Platform.isMacOS) {
    return _tryOpenCandidates(
      _desktopCandidates('lib$_libName.dylib', const [
        'macos',
        'libs',
        'libironpress.dylib',
      ]),
      platform: 'macOS',
      packagedHint: 'In a packaged Flutter macOS app, `libironpress.dylib` should be bundled automatically.',
      devHint: 'When running directly from this package checkout, ensure the repo `macos/libs` directory is available from the current working directory or `DYLD_LIBRARY_PATH`.',
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
}) {
  final errors = <String>[];
  for (final candidate in candidates) {
    try {
      return DynamicLibrary.open(candidate);
    } catch (error) {
      errors.add('  - $candidate: $error');
    }
  }

  throw StateError(
    'ironpress: failed to load the native library on $platform.\n'
    '$packagedHint\n'
    '$devHint\n'
    'Attempted locations:\n'
    '${errors.join('\n')}',
  );
}
