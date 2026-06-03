[中文](README.md) | [English](readme/README.en.md) | [日本語](readme/README.ja.md)

# Kuroko

用 Rust 编写的独立媒体播放器引擎。

Kuroko 负责播放、时序、音频输出、字幕与弹幕合成,以及原生渲染。宿主前端——Flutter、Swift,或任何带 C FFI 的环境——只需提供一个渲染 surface 并下发指令,不接触帧数据。

## 功能

- 本地文件与 HTTP Range 媒体源
- 硬件加速视频解码(macOS 上为 VideoToolbox)
- 面向 HDR/EDR 的原生 Metal / CAMetalLayer 呈现路径
- 字幕:SRT、WebVTT、ASS
- 弹幕:Bilibili XML 与 JSON-lines,带防重叠的轨道布局
- CoreAudio 音频输出
- 不透明 C ABI——可从 Swift、Dart FFI、C/C++ 或任意 Rust crate 调用

跨平台渲染(经 wgpu)正在推进中,当前以 macOS 14+ 为主。

## 构建

原生依赖、Nix 环境与示例验证见 [docs/development.md](docs/development.md)。

## 嵌入

Kuroko 提供两组 C ABI:

- **`KurokoHandle`** —— 播放控制与事件轮询。宿主自有渲染循环、或只需驱动播放时使用。
- **`KurokoPresenterHandle`** —— Kuroko 持有完整的 presenter(player、renderer、音频、overlay)。宿主提供原生 surface,并从显示定时器调用 `render_tick`。

Flutter 集成模型与 macOS HDR 嵌入策略见 [docs/flutter_embedding.md](docs/flutter_embedding.md)。

## 仓库结构

```text
crates/kuroko              播放器引擎
crates/kuroko_capi         不透明 handle 的 C ABI
crates/kuroko_ffmpeg_sys   FFmpeg 底层绑定
docs/                      架构、嵌入与开发说明
examples/                  各子系统的可运行示例
xtask/                     原生依赖编排
```

## 状态

早期阶段。macOS 播放路径已端到端可用,尚未达到生产可用。

## 许可证

MPL-2.0。原生依赖 profile 单独管理;默认 FFmpeg 构建面向 LGPL,GPL 组件为 opt-in。
