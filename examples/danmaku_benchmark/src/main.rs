use std::env;
use std::process;
use std::time::{Duration, Instant};

use kuroko::danmaku::{DanmakuLayoutConfig, DanmakuTimeline, parse_json_lines};
use kuroko::danmaku_next2::{
    Next2TimelineConfig, layout::next2_layout_frame_ref, prepare_timeline_layout,
};
use kuroko::text::TextShaper;

const DEFAULT_ITEMS: usize = 10_000;
const DEFAULT_FRAMES: usize = 600;
const DEFAULT_DURATION_SECONDS: f64 = 120.0;
const DEFAULT_FPS: f64 = 60.0;
const DEFAULT_WIDTH: f32 = 1920.0;
const DEFAULT_HEIGHT: f32 = 1080.0;

#[derive(Debug, Clone, Copy)]
struct BenchmarkConfig {
    items: usize,
    frames: usize,
    duration_seconds: f64,
    fps: f64,
    viewport_width: f32,
    viewport_height: f32,
    engine: BenchmarkEngine,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            items: DEFAULT_ITEMS,
            frames: DEFAULT_FRAMES,
            duration_seconds: DEFAULT_DURATION_SECONDS,
            fps: DEFAULT_FPS,
            viewport_width: DEFAULT_WIDTH,
            viewport_height: DEFAULT_HEIGHT,
            engine: BenchmarkEngine::Both,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BenchmarkEngine {
    Legacy,
    Next2,
    Both,
}

impl BenchmarkEngine {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "legacy" => Some(Self::Legacy),
            "next2" => Some(Self::Next2),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    fn runs_legacy(self) -> bool {
        matches!(self, Self::Legacy | Self::Both)
    }

    fn runs_next2(self) -> bool {
        matches!(self, Self::Next2 | Self::Both)
    }
}

#[derive(Debug, Clone, Copy)]
struct FrameStats {
    elapsed: Duration,
    boxes: usize,
}

fn main() {
    let config = parse_args();
    if config.items == 0 || config.frames == 0 {
        eprintln!("items and frames must be greater than zero");
        process::exit(2);
    }

    println!("Kuroko danmaku benchmark");
    println!("items: {}", config.items);
    println!("frames: {}", config.frames);
    println!("timeline duration: {:.2}s", config.duration_seconds);
    println!("target fps: {:.2}", config.fps);
    println!(
        "viewport: {:.0}x{:.0}",
        config.viewport_width, config.viewport_height
    );
    println!("engine: {:?}", config.engine);
    println!("frame budget: {:.3} ms", 1_000.0 / config.fps.max(0.001));

    let source = generate_json_lines(config.items, config.duration_seconds);

    let parse_started = Instant::now();
    let items = parse_json_lines(&source).unwrap_or_else(|error| {
        eprintln!("danmaku parse failed: {error}");
        process::exit(1);
    });
    let parse_elapsed = parse_started.elapsed();

    let build_started = Instant::now();
    let mut timeline = DanmakuTimeline::default();
    timeline.extend(items);
    let build_elapsed = build_started.elapsed();

    print_stage("parse jsonl", parse_elapsed, config.items);
    print_stage("build timeline", build_elapsed, config.items);
    if config.engine.runs_legacy() {
        benchmark_legacy(&timeline, config);
    }
    if config.engine.runs_next2() {
        benchmark_next2(&timeline, config);
    }
}

fn parse_args() -> BenchmarkConfig {
    let mut config = BenchmarkConfig::default();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--items" => {
                config.items = parse_next(&mut args, "--items");
            }
            "--frames" => {
                config.frames = parse_next(&mut args, "--frames");
            }
            "--duration" => {
                config.duration_seconds = parse_next(&mut args, "--duration");
            }
            "--fps" => {
                config.fps = parse_next(&mut args, "--fps");
            }
            "--viewport" => {
                let value = args
                    .next()
                    .unwrap_or_else(|| usage("--viewport requires WxH"));
                let (width, height) = parse_viewport(&value);
                config.viewport_width = width;
                config.viewport_height = height;
            }
            "--engine" => {
                let value = args
                    .next()
                    .unwrap_or_else(|| usage("--engine requires legacy, next2, or both"));
                config.engine = BenchmarkEngine::parse(&value)
                    .unwrap_or_else(|| usage("--engine must be legacy, next2, or both"));
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            _ => usage(&format!("unknown argument: {arg}")),
        }
    }
    config
}

fn parse_next<T>(args: &mut impl Iterator<Item = String>, name: &'static str) -> T
where
    T: std::str::FromStr,
{
    args.next()
        .unwrap_or_else(|| usage(&format!("{name} requires a value")))
        .parse::<T>()
        .unwrap_or_else(|_| usage(&format!("{name} has an invalid value")))
}

fn parse_viewport(value: &str) -> (f32, f32) {
    let Some((width, height)) = value.split_once('x') else {
        usage("--viewport must look like 1920x1080");
    };
    let width = width
        .parse::<f32>()
        .unwrap_or_else(|_| usage("--viewport width is invalid"));
    let height = height
        .parse::<f32>()
        .unwrap_or_else(|_| usage("--viewport height is invalid"));
    (width.max(1.0), height.max(1.0))
}

fn usage(message: &str) -> ! {
    eprintln!("{message}");
    print_usage();
    process::exit(2);
}

fn print_usage() {
    eprintln!(
        "usage: cargo run -p danmaku_benchmark -- [--items N] [--frames N] [--duration SECONDS] [--fps FPS] [--viewport WxH] [--engine legacy|next2|both]"
    );
}

fn benchmark_legacy(timeline: &DanmakuTimeline, config: BenchmarkConfig) {
    let shaper = TextShaper::default();
    let layout_config = DanmakuLayoutConfig {
        viewport_width: config.viewport_width,
        viewport_height: config.viewport_height,
        duration: Duration::from_secs(8),
        lane_gap: 2.0,
    };

    let mut frame_stats = Vec::with_capacity(config.frames);
    let mut checksum = 0.0f64;
    let layout_started = Instant::now();
    for index in 0..config.frames {
        let position = benchmark_position(index, config.frames, config.duration_seconds);
        let frame_started = Instant::now();
        let boxes = timeline.layout(position, layout_config, &shaper);
        let elapsed = frame_started.elapsed();
        checksum += boxes
            .iter()
            .map(|item| {
                item.x as f64
                    + item.y as f64
                    + item.width as f64
                    + item.height as f64
                    + item.item_id as f64
            })
            .sum::<f64>();
        frame_stats.push(FrameStats {
            elapsed,
            boxes: boxes.len(),
        });
    }
    print_layout_stats(
        "legacy layout frames",
        layout_started.elapsed(),
        &frame_stats,
        config.fps,
        checksum,
    );
}

fn benchmark_next2(timeline: &DanmakuTimeline, config: BenchmarkConfig) {
    let prepare_started = Instant::now();
    let prepared = prepare_timeline_layout(
        timeline,
        Next2TimelineConfig {
            width: f64::from(config.viewport_width),
            height: f64::from(config.viewport_height),
            font_size: 25.0,
            display_area: 1.0,
            scroll_duration: Duration::from_secs(8),
            allow_stacking: false,
            merge_danmaku: false,
        },
    )
    .unwrap_or_else(|error| {
        eprintln!("next2 prepare layout failed: {error}");
        process::exit(1);
    });
    print_stage(
        "next2 prepare layout",
        prepare_started.elapsed(),
        config.items,
    );

    let mut frame_stats = Vec::with_capacity(config.frames);
    let mut checksum = 0.0f64;
    let layout_started = Instant::now();
    for index in 0..config.frames {
        let position = benchmark_position(index, config.frames, config.duration_seconds);
        let frame_started = Instant::now();
        let frame = next2_layout_frame_ref(&prepared, position.as_secs_f64());
        let elapsed = frame_started.elapsed();
        checksum += frame
            .items
            .iter()
            .map(|item| item.x + item.y + item.offstage_x + item.time_seconds)
            .sum::<f64>();
        frame_stats.push(FrameStats {
            elapsed,
            boxes: frame.items.len(),
        });
    }
    print_layout_stats(
        "next2 layout frames",
        layout_started.elapsed(),
        &frame_stats,
        config.fps,
        checksum,
    );
}

fn generate_json_lines(items: usize, duration_seconds: f64) -> String {
    let mut out = String::with_capacity(items.saturating_mul(96));
    for index in 0..items {
        let time = synthetic_time(index, items, duration_seconds);
        let mode = synthetic_mode(index);
        let font_size = synthetic_font_size(index);
        let text = synthetic_text(index);
        out.push_str(&format!(
            r#"{{"id":{},"time":{:.3},"mode":"{}","font_size":{:.1},"text":"{}"}}"#,
            index + 1,
            time,
            mode,
            font_size,
            text
        ));
        out.push('\n');
    }
    out
}

fn synthetic_time(index: usize, items: usize, duration_seconds: f64) -> f64 {
    let base = index as f64 / items.max(1) as f64 * duration_seconds;
    let burst = (index % 17) as f64 * 0.012;
    (base + burst).min(duration_seconds)
}

fn synthetic_mode(index: usize) -> &'static str {
    if index % 23 == 0 {
        "top"
    } else if index % 29 == 0 {
        "bottom"
    } else {
        "scroll"
    }
}

fn synthetic_font_size(index: usize) -> f32 {
    match index % 5 {
        0 => 22.0,
        1 => 25.0,
        2 => 28.0,
        3 => 32.0,
        _ => 36.0,
    }
}

fn synthetic_text(index: usize) -> String {
    const PATTERNS: &[&str] = &[
        "danmaku",
        "native renderer",
        "Kuroko frame",
        "弹幕测试",
        "字幕同层合成",
        "HDR sample",
        "WWW",
        "long comment for collision pressure",
    ];
    format!("{} #{index}", PATTERNS[index % PATTERNS.len()])
}

fn benchmark_position(index: usize, frames: usize, duration_seconds: f64) -> Duration {
    if frames <= 1 {
        return Duration::ZERO;
    }
    let seconds = duration_seconds * index as f64 / (frames - 1) as f64;
    Duration::from_secs_f64(seconds.max(0.0))
}

fn print_stage(label: &str, elapsed: Duration, items: usize) {
    let ms = elapsed.as_secs_f64() * 1_000.0;
    let throughput = items as f64 / elapsed.as_secs_f64().max(0.000_001);
    println!("{label}: {ms:.3} ms ({throughput:.0} items/s)");
}

fn print_layout_stats(
    label: &str,
    total: Duration,
    frame_stats: &[FrameStats],
    fps: f64,
    checksum: f64,
) {
    let mut elapsed_ms = frame_stats
        .iter()
        .map(|stat| stat.elapsed.as_secs_f64() * 1_000.0)
        .collect::<Vec<_>>();
    elapsed_ms.sort_by(|a, b| a.total_cmp(b));

    let boxes_total = frame_stats.iter().map(|stat| stat.boxes).sum::<usize>();
    let boxes_max = frame_stats
        .iter()
        .map(|stat| stat.boxes)
        .max()
        .unwrap_or_default();
    let avg_ms = total.as_secs_f64() * 1_000.0 / frame_stats.len().max(1) as f64;
    let budget_ms = 1_000.0 / fps.max(0.001);

    println!(
        "{label}: {:.3} ms total, {:.3} ms avg/frame ({:.1}% of frame budget)",
        total.as_secs_f64() * 1_000.0,
        avg_ms,
        avg_ms / budget_ms * 100.0
    );
    println!(
        "layout percentiles: p50 {:.3} ms, p95 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
        percentile(&elapsed_ms, 0.50),
        percentile(&elapsed_ms, 0.95),
        percentile(&elapsed_ms, 0.99),
        elapsed_ms.last().copied().unwrap_or_default()
    );
    println!(
        "visible boxes: avg {:.1}, max {}",
        boxes_total as f64 / frame_stats.len().max(1) as f64,
        boxes_max
    );
    println!("checksum: {checksum:.3}");
}

fn percentile(sorted: &[f64], percentile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * percentile.clamp(0.0, 1.0)).round() as usize;
    sorted[index]
}
