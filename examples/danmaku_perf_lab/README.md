# Erika Danmaku Perf Lab

Controlled danmaku performance harness for Erika. This is intentionally outside
NipaPlay so danmaku density, viewport size, media time, video decode, and trace
output can be varied without Flutter/UI noise.

## Synthetic danmaku load

```sh
cargo run -p danmaku_perf_lab -- \
  --frames 600 \
  --rate 300 \
  --duration 120 \
  --pattern dense \
  --size 1920x1080
```

## Video-driven media time

```sh
cargo run -p danmaku_perf_lab -- \
  --video /path/to/video.mp4 \
  --frames 600 \
  --rate 300 \
  --duration 120 \
  --pattern dense
```

Use `--software` to force software decode when isolating CPU decode load.

## Native Metal window stress

Use release builds for renderer profiling; debug Rust is far too slow for useful
GPU-path conclusions.

```sh
cargo run --release -p danmaku_perf_lab -- \
  --window \
  --fullscreen \
  --target-fps 165 \
  --video /path/to/video.mp4 \
  --pattern scroll \
  --rate 600 \
  --duration 120 \
  --font-size 16 \
  --display-area 1.0 \
  --scroll-duration 10 \
  --stacking \
  --window-size 1600x900 \
  --hide-panel \
  --surface-scale 1.0 \
  --metrics-log /tmp/erika_lab_stress.jsonl \
  --auto-exit 18
```

Use `--uncapped` instead of `--target-fps` for raw throughput tests. In uncapped
mode the lab disables CAMetalLayer display sync and pumps frames as fast as the
main run loop can present them; this is useful for renderer stress but noisier
than a fixed refresh-rate run.

This intentionally creates more visible danmaku than DFM collision avoidance would
normally allow, so it is a renderer stress case rather than a real-world density
profile. The JSONL log is the source of truth for performance; the window is only
for visual sanity checks.

## Atlas prewarm comparison

```sh
cargo run -p danmaku_perf_lab -- \
  --frames 600 \
  --rate 300 \
  --duration 120 \
  --pattern dense \
  --prewarm-frames 720
```

## Reading the output

- `prepare_ms`: DFM+ prepare, measurement, filtering, track allocation, collision avoidance.
- `standalone_frame_layout_ms`: a direct frame query over the prepared layout.
- `render_plan_total_ms`: frame query plus glyph instance expansion and atlas snapshot access.
- `current_metal_draws`: estimated current Metal danmaku draw calls with per-glyph shadow/outline/fill.
- `batched_target_draws`: estimated draw calls after batching shadow/outline/fill passes.
- `atlas_changes`: number of atlas version changes during sampled frames.
- `draw_call_reduction_target`: current draw calls divided by expected batched draw calls.

Window JSONL fields worth watching:

- `fps`, `tick_ms`, `pump_ms`, `render_ms`: overall frame health.
- `video_pump_ms`, `danmaku_plan_ms`: presenter-side decode/import and render-plan cost.
- `danmaku_vertex_build_ms`, `danmaku_vertex_copy_ms`, `danmaku_encode_ms`: Metal danmaku pass CPU cost.
- `draw_items_per_new_pass`: glyph instances per newly rendered danmaku pass.
- `danmaku_vertex_bytes`: bytes written into the Metal instance buffer for the current frame.

This lab is the baseline for Metal/WGPU danmaku batching work.
