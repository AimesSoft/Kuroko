# Flutter Embedding

Erika is not a Flutter video renderer. Flutter is an optional host UI.
The player owns decode, timing, native rendering, subtitles, danmaku, audio, and
HDR presentation.

This document describes how a Flutter application should use the current v0
foundation.

## API Families

There are two C ABI entrypoint families:

- `ErikaHandle`: control and event API. Use this when the host owns its own
  presenter loop or only wants to probe/control playback.
- `ErikaPresenterHandle`: presenter-owned API. Use this when Erika should
  own `Player + MetalRenderer + CoreAudio` and the host only supplies a native
  surface plus a display-tick callback.

Both families are declared in
`crates/erika_capi/include/erika.h`.

## macOS HDR Path

The macOS HDR path should use a native Metal-backed surface, not Flutter
Texture. This matches the direction proven by NipaPlay's earlier macOS HDR PR:
Flutter/AppKit view composition can show video but is vulnerable to black
flicker, while a window-hosted native layer with a transparent Flutter region is
the stable HDR direction.

The intended plugin shape is:

1. Dart creates a normal Flutter widget that reserves a rectangle for video.
2. The macOS plugin creates a window-hosted native `NSView`/`CAMetalLayer` as a
   sibling or underlay of Flutter's content.
3. Flutter paints that widget region transparent or otherwise leaves a hole for
   the native video layer.
4. The plugin sends geometry updates to native code with a surface generation
   number.
5. Native code attaches the `CAMetalLayer` pointer through
   `erika_presenter_attach_metal_layer`.
6. Native code drives `erika_presenter_render_tick` from a display timer such as
   `CVDisplayLink`, AppKit display callbacks, or a platform timer.
7. Dispose/hide calls only detach the surface if their generation still matches
   the currently attached surface.

The important ownership rule is simple: Flutter owns layout and controls;
Erika owns the video plane, subtitle plane, danmaku plane, audio, and
timing.

## Minimal macOS Presenter Flow

```c
ErikaPresenterHandle *presenter = erika_presenter_create();
erika_presenter_attach_metal_layer(
    presenter,
    (uint64_t)cametal_layer,
    width,
    height,
    backing_scale);
erika_presenter_open(presenter, "/path/to/media.mp4");
erika_presenter_play(presenter);

// On every display tick:
ErikaPresenterStats stats;
erika_presenter_render_tick(presenter, host_time_seconds, &stats);

// On resize:
erika_presenter_resize_surface(presenter, width, height, backing_scale);

// On dispose:
erika_presenter_detach_surface(presenter);
erika_presenter_destroy(presenter);
```

The current native demo already uses this pattern:
`examples/macos_native_demo` creates the AppKit window/layer, while
`PresenterRuntime` owns playback, Metal rendering, overlay composition, and
CoreAudio output.

## Flutter Texture Path

Flutter Texture is allowed, but it is a lower-capability compatibility path.

Texture output is useful for:

- SDR fallback.
- Platforms where native view composition is not ready.
- Test surfaces or constrained embedding environments.

Texture output is not the preferred HDR/EDR route because video enters
Flutter's compositor. On Apple platforms, that means it cannot be treated as
equivalent to a renderer-owned Metal surface. The C ABI already has
`erika_attach_flutter_texture` so the host API shape is reserved, but the
production renderer path should keep HDR playback on the native presenter.

## WGPU Fallback

The Apple HDR path remains native Metal first. `wgpu` is the cross-platform
fallback direction for Windows, Linux, Android, and non-HDR or less specialized
paths.

The current `renderer::wgpu` module intentionally only defines the lifecycle
boundary:

- attach a platform surface handle;
- resize it;
- render a test frame;
- detach it safely.

The next wgpu milestone should add the real dependency, surface creation,
WGSL YCbCr/P010 conversion, subtitle/danmaku batches, and platform-specific
texture import or upload strategies.

## Difference From The Earlier NipaPlay macOS HDR PR

The composition strategy is the same where it matters: avoid sending HDR video
through Flutter's compositor and reserve native screen real estate for a Metal
surface.

The ownership is different:

- Earlier NipaPlay work proved the host-side macOS HDR embedding strategy.
- Erika moves decode, frame import, Metal rendering, CoreAudio, subtitle
  overlay, danmaku layout, and timing into a standalone Rust player.
- Flutter will call a narrow C ABI instead of owning playback internals.

That makes the native-surface approach a normal player-kernel integration:
Flutter remains the app shell, while Erika behaves like a platform media
engine that presents directly into native surfaces.
