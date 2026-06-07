Pod::Spec.new do |s|
  s.name             = 'erika_flutter'
  s.version          = '0.0.1'
  s.summary          = 'Flutter embedder glue for the Erika Rust media engine.'
  s.description      = <<-DESC
Flutter iOS plugin that hosts a CAMetalLayer and drives Erika through its C ABI.
                       DESC
  s.homepage         = 'https://github.com/AimesSoft/Erika'
  s.license          = { :type => 'MPL-2.0' }
  s.author           = { 'AimesSoft' => 'dev@aimesoft.com' }
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'Flutter'
  s.platform = :ios, '13.0'
  s.swift_version = '5.0'
  s.script_phase = {
    :name => 'Build Erika C ABI',
    :execution_position => :before_compile,
    :script => <<-SCRIPT
set -eu

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

PLUGIN_IOS_DIR="$(cd "$PODS_TARGET_SRCROOT" && pwd -P)"
ERIKA_ROOT="$(cd "$PLUGIN_IOS_DIR/../../.." && pwd -P)"
ARCH="${CURRENT_ARCH:-}"
if [ -z "$ARCH" ] || [ "$ARCH" = "undefined_arch" ]; then
  ARCH="${ARCHS%% *}"
fi

case "${PLATFORM_NAME:-iphoneos}" in
  iphoneos)
    RUST_TARGET="aarch64-apple-ios"
    ;;
  iphonesimulator)
    if [ "$ARCH" = "x86_64" ]; then
      RUST_TARGET="x86_64-apple-ios"
    else
      RUST_TARGET="aarch64-apple-ios-sim"
    fi
    ;;
  *)
    echo "error: unsupported Erika iOS platform: ${PLATFORM_NAME:-unknown}" >&2
    exit 1
    ;;
esac

if [ "${CONFIGURATION:-Debug}" = "Debug" ]; then
  CARGO_PROFILE="debug"
  CARGO_ARGS=""
else
  CARGO_PROFILE="release"
  CARGO_ARGS="--release"
fi

if [ -n "${ERIKA_IOS_CAPI_STATICLIB:-}" ]; then
  LIB_SOURCE="$ERIKA_IOS_CAPI_STATICLIB"
else
  LIB_SOURCE="$ERIKA_ROOT/target/$RUST_TARGET/$CARGO_PROFILE/liberika_capi.a"
  if [ ! -f "$LIB_SOURCE" ]; then
    echo "Building Erika C ABI for $RUST_TARGET ($CARGO_PROFILE)"
    (cd "$ERIKA_ROOT" && cargo build -p erika_capi --target "$RUST_TARGET" $CARGO_ARGS)
  fi
fi

if [ ! -f "$LIB_SOURCE" ]; then
  echo "error: Erika C ABI static library not found: $LIB_SOURCE" >&2
  echo "       Build it with: cargo build -p erika_capi --target $RUST_TARGET $CARGO_ARGS" >&2
  exit 1
fi

mkdir -p "$PODS_TARGET_SRCROOT/native"
cp "$LIB_SOURCE" "$PODS_TARGET_SRCROOT/native/liberika_capi.a"
    SCRIPT
  }
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
    'OTHER_LDFLAGS' => '$(inherited) -force_load "$(PODS_TARGET_SRCROOT)/native/liberika_capi.a" -framework AVFoundation -framework AudioToolbox -framework QuartzCore -framework Metal -framework CoreVideo -framework CoreMedia -framework VideoToolbox -framework CoreFoundation -framework CoreGraphics -framework Foundation -liconv -lbz2 -lz',
  }
end
