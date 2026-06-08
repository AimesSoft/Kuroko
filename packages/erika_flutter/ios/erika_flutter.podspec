Pod::Spec.new do |s|
  erika_cabi_symbols = %w[
    erika_danmaku_track_info_free
    erika_presenter_add_danmaku_track_file
    erika_presenter_add_danmaku_track_json
    erika_presenter_add_external_subtitle
    erika_presenter_attach_metal_layer
    erika_presenter_clear_danmaku
    erika_presenter_close
    erika_presenter_create
    erika_presenter_create_with_output_mode
    erika_presenter_danmaku_tracks
    erika_presenter_destroy
    erika_presenter_detach_surface
    erika_presenter_get_danmaku_config
    erika_presenter_load_danmaku_file
    erika_presenter_load_danmaku_json
    erika_presenter_open
    erika_presenter_pause
    erika_presenter_play
    erika_presenter_poll_event
    erika_presenter_remove_danmaku_track
    erika_presenter_remove_subtitle_track
    erika_presenter_render_tick
    erika_presenter_resize_surface
    erika_presenter_seek
    erika_presenter_select_audio_track
    erika_presenter_select_subtitle_track
    erika_presenter_set_danmaku_block_words_json
    erika_presenter_set_danmaku_config_ptr
    erika_presenter_set_danmaku_enabled
    erika_presenter_set_danmaku_font
    erika_presenter_set_danmaku_global_offset
    erika_presenter_set_danmaku_track_enabled
    erika_presenter_set_danmaku_track_offset
    erika_presenter_set_playback_rate
    erika_presenter_set_volume
    erika_presenter_stop
    erika_presenter_track_selection
    erika_presenter_tracks
    erika_track_info_free
  ]
  erika_cabi_undefined_flags = erika_cabi_symbols
    .map { |symbol| "-Wl,-u,_#{symbol}" }
    .join(' ')

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
    :output_files => ['${PODS_TARGET_SRCROOT}/native/liberika_capi.a'],
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

if [ -n "${ERIKA_IOS_CAPI_PROFILE:-}" ]; then
  CARGO_PROFILE="$ERIKA_IOS_CAPI_PROFILE"
elif [ "${CONFIGURATION:-Debug}" = "Release" ]; then
  CARGO_PROFILE="release"
else
  CARGO_PROFILE="debug"
fi

if [ "$CARGO_PROFILE" = "release" ]; then
  CARGO_ARGS="--release"
elif [ "$CARGO_PROFILE" = "debug" ]; then
  CARGO_ARGS=""
else
  echo "error: unsupported ERIKA_IOS_CAPI_PROFILE=$CARGO_PROFILE" >&2
  exit 1
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
    'OTHER_LDFLAGS' => "$(inherited) \"$(PODS_TARGET_SRCROOT)/native/liberika_capi.a\" #{erika_cabi_undefined_flags} -framework AVFoundation -framework AudioToolbox -framework QuartzCore -framework Metal -framework CoreVideo -framework CoreMedia -framework VideoToolbox -framework CoreFoundation -framework CoreGraphics -framework Foundation -liconv -lbz2 -lz",
  }
end
