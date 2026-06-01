[中文](README.md) | [English](readme/README.en.md) | [日本語](readme/README.ja.md)

# Kuroko

Kuroko 是一个用 Rust 编写的独立媒体播放器引擎。

Kuroko 负责播放控制、时序同步、原生渲染、字幕、弹幕、音频输出，以及面向 HDR/EDR 的呈现路径。

当前首要实现目标是 macOS 14+ 与原生 Metal presenter。长期方向是优先使用各平台原生渲染器，并在原生 API 不适合的场景下提供 wgpu fallback。

## 当前状态

Kuroko 仍处于早期引擎基础阶段，还不是完整的终端播放器。当前代码树已经包含一条可运行的 macOS 播放路径：

- 通过 `xtask` 从源码构建 FFmpeg 7.1.1，默认 profile 面向 LGPL。
- 基于 Kuroko 本地文件与 HTTP Range source abstraction 的自定义 FFmpeg `AVIOContext`。
- Probe、demux、软件解码、音频解码与 seek wrappers。
- macOS 上的 VideoToolbox HEVC Main 10 硬件解码。
- 通过 `CVMetalTextureCache` 将 `CVPixelBuffer` 零拷贝导入为 Metal texture。
- 可将 NV12/P010 frame 绘制到 `CAMetalLayer` 的 Metal renderer。
- 后端无关的 RendererCore 雏形，包括 source/target color state、render graph passes、tone mapping policy 与 scaler policy。
- Metal 中的原生字幕 overlay plane blending。
- 早期字幕解析、feature-gated libass 静态链接 renderer、弹幕解析/布局，以及共享 text-measurement 边界。
- 基于 interleaved f32 PCM ring buffer 的 CoreAudio 输出，包含音频队列 PTS clock snapshot。
- 带有有界视频/音频队列、第一版 audio-output clock discipline 与 display-vsync quantizer 的 `Player` worker thread。
- 面向嵌入方的 `PresenterRuntime`，可由 Kuroko 接管 playback、Metal rendering、overlays 与 CoreAudio。
- `kuroko_capi` 提供 opaque C handles，可供 Rust、C/C++、Swift/ObjC，以及未来 Dart FFI 使用。
- 面向未来跨平台 fallback 的 wgpu renderer lifecycle 边界。

仍在进行中：生产级 HDR/EDR 输出、成熟的 tone mapping/scaler 质量、ICC/display profile 处理、更精确的 CoreAudio host-time/device-time 对齐、将 libass bitmap 更深地接入 renderer graph、原生弹幕 glyph atlas/batching、真正的 wgpu compositor 实现，以及打包好的 Flutter plugin glue。

Kuroko 运行时不依赖 mpv、gpu-next 或 libplacebo。渲染方向是干净的 Rust 实现，但会参考同类现代媒体渲染器中的设计：显式 frame import、color state、shader graph、tone mapping、scaling、overlay composition、dithering 与 native presentation。

## 仓库结构

```text
crates/kuroko              主播放器引擎 crate
crates/kuroko_capi         Opaque-handle C ABI
crates/kuroko_ffmpeg_sys   低层 FFmpeg generated bindings
docs/                      架构与嵌入说明
examples/                  Probe、decode、playback、Metal、CoreAudio、C ABI demos
xtask/                     原生依赖编排入口
```

## 原生依赖

Kuroko 使用源码构建的原生依赖，以便控制发布产物与许可证边界。默认 profile 是 `lgpl`；`gpl-full` 保留给未来可选构建。

```sh
cargo run -p xtask -- deps plan --profile lgpl
cargo run -p xtask -- deps build --profile lgpl
cargo run -p xtask -- deps build --all --profile lgpl
cargo run -p xtask -- deps status --profile lgpl
cargo run -p xtask -- check license
```

当前 FFmpeg 构建路径已经启用。传入 `--all` 时，`xtask` 还会构建 libass、HarfBuzz、FreeType 与 FriBidi；`kuroko` 的 `libass` feature 会静态链接这些依赖并启用真实 ASS bitmap renderer。

## 开发环境

仓库包含一个 Nix flake：

```sh
nix develop
```

无论是否进入 Nix shell，`xtask` 都是原生依赖相关开发工作的入口。

## 快速验证

`$SAMPLE` 可以使用任意本地媒体文件；如果要验证 macOS 硬件路径，HDR HEVC Main 10 / BT.2020 / PQ sample 会更有代表性。

```sh
export SAMPLE="/path/to/sample.mp4"

cargo test --workspace
cargo test -p kuroko --features libass
cargo run -p ffmpeg_probe -- "$SAMPLE"
cargo run -p ffmpeg_probe_source -- "$SAMPLE" file
cargo run -p ffmpeg_decode -- "$SAMPLE" 4
cargo run -p ffmpeg_audio_decode -- "$SAMPLE" 4
cargo run -p ffmpeg_videotoolbox -- "$SAMPLE" 2
cargo run -p metal_import_videotoolbox -- "$SAMPLE"
cargo run -p render_pipeline_plan
cargo run -p metal_overlay_check
cargo run -p player_tick -- "$SAMPLE" 6
cargo run -p capi_smoke -- "$SAMPLE"
cargo run -p coreaudio_smoke -- "$SAMPLE" 2
cargo run -p macos_native_demo -- --smoke-seconds 1.5 "$SAMPLE"
```

`macos_native_demo` 会打开原生 AppKit window，并驱动由库拥有的 presenter runtime。smoke mode 会自动退出，便于重复验证。

## Flutter 与 HDR

Flutter 现在已经可以通过 C ABI 调用 Kuroko 的 player lifetime、media open、playback commands、seeking、state queries、event polling 与 native surface attachment。Flutter plugin 层本身还没有打包完成。

当前有两组 C ABI entrypoint：

- `KurokoHandle`：控制/事件 API，适合由宿主管理呈现的场景。
- `KurokoPresenterHandle`：macOS presenter API，由 Kuroko 持有 `Player`、`MetalRenderer`、CoreAudio output 与 `render_tick()`。

macOS HDR 嵌入的推荐路径是原生 Metal-backed surface：Flutter widget 预留一个矩形区域，macOS plugin 托管原生 `CAMetalLayer`，Kuroko 通过 `kuroko_presenter_attach_metal_layer` 与 `kuroko_presenter_render_tick` 直接渲染到该 layer。

Flutter Texture 可以作为 SDR 或受限嵌入场景下的兼容路径，但不应被视为 Apple HDR/EDR 路径，因为视频会进入 Flutter compositor。具体嵌入模型以及它和早期 NipaPlay macOS HDR 工作的关系，见 [docs/flutter_embedding.md](docs/flutter_embedding.md)。

## 许可证

Rust workspace 当前采用 MPL-2.0 许可。原生依赖 profile 由 `xtask` 单独管理；默认 FFmpeg profile 面向 LGPL，并将 GPL 组件保持为 opt-in。
