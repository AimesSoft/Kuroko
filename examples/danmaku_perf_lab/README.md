# Kuroko Danmaku Perf Lab

Controlled danmaku performance harness for Kuroko. This is intentionally outside
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

This lab is the baseline for Metal/WGPU danmaku batching work.
