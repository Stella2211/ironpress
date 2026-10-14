Pod::Spec.new do |s|
  s.name             = 'ironpress'
  s.version          = '0.3.0'
  s.summary          = 'High-performance image compression powered by Rust'
  s.description      = <<-DESC
    Flutter plugin providing Rust-powered image compression using
    mozjpeg (trellis quantization) and oxipng. Delivers consistent,
    superior compression across all platforms.
  DESC
  s.homepage         = 'https://github.com/rickyirfandi/ironpress'
  s.license          = { :type => 'MIT', :file => '../LICENSE' }
  s.author           = { 'Ricky Irfandi' => 'dev@rickyirfandi.dev' }
  s.source           = { :path => '.' }

  # Keep in sync with ironpress/Package.swift and scripts/build.sh.
  s.ios.deployment_target = '15.0'
  s.osx.deployment_target = '12.0'

  # Shared with Swift Package Manager (ironpress/Package.swift). The
  # xcframework contains dynamic frameworks for iOS device, iOS simulator,
  # and macOS (universal). CocoaPods embeds and re-signs it in the app.
  s.vendored_frameworks = 'ironpress/Frameworks/ironpress_native.xcframework'

  s.source_files = 'ironpress/Sources/ironpress/**/*.swift'
  s.ios.dependency 'Flutter'
  s.osx.dependency 'FlutterMacOS'

  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES' }
  s.swift_version = '5.0'

  # Privacy manifest (required since Xcode 15.3)
  s.resource_bundles = {
    'ironpress_privacy' => ['ironpress/Sources/ironpress/PrivacyInfo.xcprivacy']
  }
end
