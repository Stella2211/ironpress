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

  s.ios.deployment_target = '13.0'

  # The precompiled xcframework is placed at:
  #   ios/Frameworks/ironpress.xcframework/
  s.vendored_frameworks = 'Frameworks/ironpress.xcframework'

  # Minimal Swift file to satisfy Flutter's plugin registration
  s.source_files = 'Classes/**/*'
  s.dependency 'Flutter'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Strip debug symbols in release for smaller binary
    'STRIP_INSTALLED_PRODUCT' => 'YES',
    'DEAD_CODE_STRIPPING' => 'YES',
  }
  s.swift_version = '5.0'

  # Privacy manifest (required since Xcode 15.3)
  s.resource_bundles = {
    'ironpress_privacy' => ['Resources/PrivacyInfo.xcprivacy']
  }
end
