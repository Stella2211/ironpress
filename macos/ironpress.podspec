Pod::Spec.new do |s|
  s.name             = 'ironpress'
  s.version          = '0.3.0'
  s.summary          = 'High-performance image compression powered by Rust'
  s.description      = <<-DESC
    Flutter plugin providing Rust-powered image compression.
  DESC
  s.homepage         = 'https://github.com/rickyirfandi/ironpress'
  s.license          = { :type => 'MIT', :file => '../LICENSE' }
  s.author           = { 'Ricky Irfandi' => 'dev@rickyirfandi.dev' }
  s.source           = { :path => '.' }

  s.osx.deployment_target = '10.15'

  # Precompiled universal dylib (arm64 + x86_64)
  s.vendored_libraries = 'libs/libironpress.dylib'

  s.dependency 'FlutterMacOS'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
  }
  s.swift_version = '5.0'
end
