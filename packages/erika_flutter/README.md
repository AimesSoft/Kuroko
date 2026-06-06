# erika_flutter

Experimental Flutter embedder glue for Erika.

The plugin keeps Dart out of the hot path:

- Dart exposes low-frequency player commands and event streams.
- The macOS plugin owns `NSView`/`CAMetalLayer` lifecycle.
- The iOS plugin owns `UIView`/`CAMetalLayer` lifecycle and drives the same
  Apple Metal presenter path.
- Erika owns playback, rendering, audio, timing, and overlays through
  `ErikaPresenterHandle`.

For local development the macOS plugin loads Erika through `dlopen`.
Set `ERIKA_CAPI_DYLIB` to override the dynamic library path. If unset, the
plugin tries the app bundle, the executable directory, and then
`/Users/sakiko/Desktop/Erika/target/debug/liberika_capi.dylib`.

## Output mode

`ErikaPlayer()` lets the macOS plugin choose SDR or Apple EDR from the current
screen and environment. To force EDR from Dart:

```dart
final player = ErikaPlayer(
  outputMode: ErikaOutputMode.appleEdr,
  edrHeadroom: 4.0,
);
```

Use `ErikaOutputMode.sdr` to force SDR output.

## iOS status

iOS support is in the first integration stage. The Rust presenter and C ABI now
compile for `aarch64-apple-ios`, VideoToolbox frames are imported through the
same `CVPixelBuffer` -> Metal path as macOS, and the Flutter plugin exposes a
`UiKitView` backed by `CAMetalLayer`.

Current limitations:

- The app must link the Erika C ABI static library or XCFramework so the iOS
  plugin can resolve `erika_presenter_*` symbols from the main executable.
- iOS audio output is still a buffered placeholder; replace it with an
  `AVAudioEngine` backend before treating playback as complete.
- The macOS window-overlay surface is not implemented on iOS. Use
  `ErikaVideoView` for the first iOS HDR path.
