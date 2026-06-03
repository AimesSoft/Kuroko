# Development

## Native dependencies

Kuroko builds its native dependencies from source so the release artifacts and
license boundary stay under our control. `xtask` is the entry point. The default
profile is `lgpl`; `gpl-full` is reserved for an opt-in build.

```sh
cargo run -p xtask -- deps plan    --profile lgpl
cargo run -p xtask -- deps build   --profile lgpl
cargo run -p xtask -- deps build --all --profile lgpl
cargo run -p xtask -- deps status  --profile lgpl
cargo run -p xtask -- check license
```

`deps build` compiles FFmpeg 7.1.1. With `--all` it also builds libass,
HarfBuzz, FreeType, and FriBidi; enabling the `libass` feature on `kuroko`
statically links them and turns on the real ASS bitmap renderer.

## Environment

A Nix flake is provided:

```sh
nix develop
```

`xtask` is the entry point for native-dependency work whether or not you use the
Nix shell.

## Examples

Most subsystems have a runnable example. `$SAMPLE` can be any local media file;
an HDR HEVC Main 10 / BT.2020 / PQ clip exercises the macOS hardware path.

```sh
export SAMPLE="/path/to/sample.mp4"

cargo test --workspace
cargo test -p kuroko --features libass

cargo run -p ffmpeg_probe          -- "$SAMPLE"
cargo run -p ffmpeg_decode         -- "$SAMPLE" 4
cargo run -p ffmpeg_videotoolbox   -- "$SAMPLE" 2
cargo run -p player_tick           -- "$SAMPLE" 6
cargo run -p coreaudio_smoke       -- "$SAMPLE" 2
cargo run -p capi_smoke            -- "$SAMPLE"

# Native AppKit window driving the library-owned presenter (smoke mode exits on its own)
cargo run -p macos_native_demo -- --smoke-seconds 1.5 "$SAMPLE"
cargo run -p macos_native_demo --features libass -- \
    --ass-subtitle "/path/to/subtitle.ass" --smoke-seconds 1.5 "$SAMPLE"
```

### wgpu backend

The wgpu renderer is behind the `wgpu` feature. These examples render off-screen
to a PNG or to a real window, and run without a media file:

```sh
cargo run -p wgpu_video_png      # color bars through the WGSL video pipeline
cargo run -p wgpu_overlay_png    # video + subtitle overlay compositing
cargo run -p wgpu_window_check   # present to a real CAMetalLayer window
cargo run -p wgpu_decode_png -- "$SAMPLE" out.png 8   # decode a real frame, render via wgpu
```
