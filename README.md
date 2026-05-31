# Kuroko

Kuroko is a standalone Rust-first media player engine. It owns playback,
timing, native rendering, subtitles, danmaku, audio output, and HDR
presentation. Flutter is an optional host UI, not the video renderer.

The first implementation target is macOS 14+ with a native Metal presenter.
The long-term direction is full-platform support through platform-native
renderers first and a wgpu fallback path where native APIs are not the right
fit.

## Status

Kuroko is an early engine foundation, not a finished end-user player. The
current tree already contains a working macOS playback path:

- FFmpeg 7.1.1 source-built through `xtask` with an LGPL default profile.
- Custom FFmpeg `AVIOContext` backed by Kuroko's local-file and HTTP range
  source abstraction.
- Probe, demux, software decode, audio decode, and seek wrappers.
- VideoToolbox HEVC Main 10 hardware decode on macOS.
- Zero-copy `CVPixelBuffer` to Metal texture import through
  `CVMetalTextureCache`.
- A Metal renderer that draws NV12/P010 frames into `CAMetalLayer`.
- Native subtitle overlay plane blending in Metal.
- Early subtitle parsing, danmaku parsing/layout, and shared text-measurement
  boundaries.
- CoreAudio output backed by an interleaved f32 PCM ring buffer.
- `Player` worker thread with bounded video and audio queues.
- `PresenterRuntime` for embedders that want Kuroko to own playback, Metal
  rendering, overlays, and CoreAudio.
- `kuroko_capi` exposing opaque C handles for Rust, C/C++, Swift/ObjC, and
  future Dart FFI usage.
- A wgpu renderer lifecycle boundary for the future cross-platform fallback.

Still upcoming: final HDR/EDR tone mapping, audio-master A/V sync, libass
bitmap rendering, native danmaku glyph atlas/batching, real wgpu compositor
implementation, and packaged Flutter plugin glue.

## Repository Layout

```text
crates/kuroko              Main player engine crate
crates/kuroko_capi         Opaque-handle C ABI
crates/kuroko_ffmpeg_sys   Low-level generated FFmpeg bindings
docs/                      Architecture and embedding notes
examples/                  Probe, decode, playback, Metal, CoreAudio, C ABI demos
xtask/                     Native dependency orchestration
```

## Native Dependencies

Kuroko uses source-built native dependencies for release control and license
clarity. The default profile is `lgpl`; `gpl-full` is reserved for optional
future builds.

```sh
cargo run -p xtask -- deps plan --profile lgpl
cargo run -p xtask -- deps build --profile lgpl
cargo run -p xtask -- deps status --profile lgpl
cargo run -p xtask -- check license
```

The FFmpeg build is currently active. libass, HarfBuzz, FreeType, and FriBidi
are pinned and prepared by `xtask`; their full static link path lands with the
subtitle/text renderer milestone.

## Development Shell

The repository includes a Nix flake:

```sh
nix develop
```

Inside or outside the Nix shell, `xtask` remains the developer entrypoint for
native dependency work.

## Quick Verification

Use any local media file for `$SAMPLE`; an HDR HEVC Main 10 / BT.2020 / PQ
sample is useful for the macOS hardware path.

```sh
export SAMPLE="/path/to/sample.mp4"

cargo test --workspace
cargo run -p ffmpeg_probe -- "$SAMPLE"
cargo run -p ffmpeg_probe_source -- "$SAMPLE" file
cargo run -p ffmpeg_decode -- "$SAMPLE" 4
cargo run -p ffmpeg_audio_decode -- "$SAMPLE" 4
cargo run -p ffmpeg_videotoolbox -- "$SAMPLE" 2
cargo run -p metal_import_videotoolbox -- "$SAMPLE"
cargo run -p metal_overlay_check
cargo run -p player_tick -- "$SAMPLE" 6
cargo run -p capi_smoke -- "$SAMPLE"
cargo run -p coreaudio_smoke -- "$SAMPLE" 2
cargo run -p macos_native_demo -- --smoke-seconds 1.5 "$SAMPLE"
```

`macos_native_demo` opens a native AppKit window and drives the library-owned
presenter runtime. The smoke mode exits automatically for repeatable validation.

## Flutter And HDR

Flutter can call the C ABI today for player lifetime, media open, playback
commands, seeking, state queries, event polling, and native surface attachment.
The Flutter plugin layer itself is not packaged yet.

There are two C ABI entrypoint families:

- `KurokoHandle`: control/event API for hosts that manage presentation.
- `KurokoPresenterHandle`: macOS presenter API that owns `Player`,
  `MetalRenderer`, CoreAudio output, and `render_tick()`.

For macOS HDR embedding, the preferred path is a native Metal-backed surface:
the Flutter widget reserves a rectangle, the macOS plugin hosts a native
`CAMetalLayer`, and Kuroko renders directly into that layer through
`kuroko_presenter_attach_metal_layer` and `kuroko_presenter_render_tick`.

Flutter Texture is allowed as a compatibility path for SDR or constrained
embedding cases, but it should not be treated as the Apple HDR/EDR path because
video then enters Flutter's compositor. See `docs/flutter_embedding.md` for the
concrete embedding model and its relationship to the earlier NipaPlay macOS HDR
work.

## License

The Rust workspace is currently licensed under MPL-2.0. Native dependency
profiles are managed separately through `xtask`; the default FFmpeg profile is
LGPL-oriented and keeps GPL components opt-in.
