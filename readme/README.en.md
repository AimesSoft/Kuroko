[中文](../README.md) | [English](README.en.md) | [日本語](README.ja.md)

# Kuroko

A standalone media player engine written in Rust.

Kuroko owns playback, timing, audio output, subtitle and danmaku overlay, and native rendering. The host frontend -- Flutter, Swift, or anything with a C FFI -- supplies a surface and issues commands. It does not touch frames.

## Capabilities

- Local file and HTTP range media sources
- Hardware-accelerated video decode (VideoToolbox on macOS)
- Native Metal/CAMetalLayer presentation path aimed at HDR/EDR
- Subtitle support: SRT, WebVTT, ASS
- Danmaku: Bilibili XML and JSON-lines, with collision-aware lane layout
- Audio output through CoreAudio
- Opaque C ABI -- callable from Swift, Dart FFI, C/C++, or any Rust crate

A wgpu rendering backend is in progress for cross-platform support; macOS 14+ is the current focus.

## Building

Native dependencies, the Nix environment, and the example commands live in [`docs/development.md`](../docs/development.md).

## Embedding

Kuroko exposes two C ABI families:

- **`KurokoHandle`** — player control and event polling. Use this when the host owns its own render loop or only needs to drive playback.
- **`KurokoPresenterHandle`** — Kuroko owns the full presenter stack (player, renderer, audio, overlays). The host supplies a native surface and calls `render_tick` from its display timer.

See [`docs/flutter_embedding.md`](../docs/flutter_embedding.md) for the Flutter integration model and the macOS HDR embedding strategy.

## Repository layout

```text
crates/kuroko              the player engine
crates/kuroko_capi         opaque-handle C ABI
crates/kuroko_ffmpeg_sys   low-level FFmpeg bindings
docs/                      architecture, embedding, and development notes
examples/                  runnable per-subsystem examples
xtask/                     native-dependency orchestration
```

## Status

Early foundation. The macOS playback path is working end-to-end. Not ready for production use.

## License

MPL-2.0. Native dependency profiles are managed separately; the default FFmpeg build is LGPL-oriented with GPL components opt-in.
