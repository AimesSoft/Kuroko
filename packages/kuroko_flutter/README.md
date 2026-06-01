# kuroko_flutter

Experimental Flutter embedder glue for Kuroko.

The plugin keeps Dart out of the hot path:

- Dart exposes low-frequency player commands and event streams.
- The macOS plugin owns `NSView`/`CAMetalLayer` lifecycle.
- Kuroko owns playback, rendering, audio, timing, and overlays through
  `KurokoPresenterHandle`.

For local development the macOS plugin loads Kuroko through `dlopen`.
Set `KUROKO_CAPI_DYLIB` to override the dynamic library path. If unset, the
plugin tries the app bundle, the executable directory, and then
`/Users/sakiko/Desktop/Kuroko/target/debug/libkuroko_capi.dylib`.

## Output mode

`KurokoPlayer()` lets the macOS plugin choose SDR or Apple EDR from the current
screen and environment. To force EDR from Dart:

```dart
final player = KurokoPlayer(
  outputMode: KurokoOutputMode.appleEdr,
  edrHeadroom: 4.0,
);
```

Use `KurokoOutputMode.sdr` to force SDR output.
