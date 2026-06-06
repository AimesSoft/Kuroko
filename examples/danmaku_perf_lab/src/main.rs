use std::env;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use kuroko::MediaRequest;
use kuroko::danmaku::{
    DanmakuColor, DanmakuItem, DanmakuLayoutConfig, DanmakuMode, DanmakuShadowStyle,
    DanmakuTimeline, DanmakuViewport, DfmLayoutEngine,
};
use kuroko::playback::{PlaybackSessionConfig, VideoDecodePreference, VideoPlaybackEngine};

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let options = parse_args(&args).unwrap_or_else(|error| {
        eprintln!("{error}");
        eprintln!(
            "usage: cargo run -p danmaku_perf_lab -- [--danmaku PATH] [--video PATH] [--software] \
             [--items N|--rate N] [--duration SECONDS] [--frames N] [--fps N] [--size WxH] \
             [--prewarm-frames N] \
             [--font-size N] [--display-area N] [--scroll-duration N] [--pattern mixed|scroll|fixed|dense]"
        );
        process::exit(2);
    });

    match run(options) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("danmaku perf lab failed: {error}");
            process::exit(1);
        }
    }
}

fn run(options: PerfOptions) -> Result<(), String> {
    let timeline = load_or_generate_timeline(&options)?;
    let config = DanmakuLayoutConfig {
        font_size: options.font_size,
        display_area: options.display_area,
        scroll_duration_seconds: options.scroll_duration_seconds,
        outline_width: options.outline_width,
        shadow_style: options.shadow_style,
        merge_duplicates: options.merge_duplicates,
        allow_stacking: options.allow_stacking,
        ..DanmakuLayoutConfig::default()
    };
    let viewport = DanmakuViewport::new(options.width, options.height);
    let mut engine = DfmLayoutEngine::new(timeline, config);

    println!("Kuroko danmaku perf lab");
    println!(
        "mode: {}",
        if options.video_uri.is_some() {
            "video-driven"
        } else {
            "synthetic-clock"
        }
    );
    println!(
        "viewport: {}x{} frames={} fps={:.3} duration={:.3}s",
        viewport.width, viewport.height, options.frames, options.fps, options.duration_seconds
    );
    println!(
        "danmaku: items={} rate={:.1}/s pattern={:?} font={:.1} display_area={:.2} scroll_dur={:.2}s",
        options.item_count,
        options.rate,
        options.pattern,
        options.font_size,
        options.display_area,
        options.scroll_duration_seconds
    );

    let prepare_started = Instant::now();
    let prepared = engine.prepare(viewport, 1);
    let prepare_elapsed = prepare_started.elapsed();
    println!(
        "prepare: {:.3}ms prepared_items={}",
        ms(prepare_elapsed),
        prepared.items().len()
    );

    let prewarm = prewarm_atlas(&mut engine, viewport, &options);
    if prewarm.frames > 0 {
        println!(
            "prewarm: {:.3}ms frames={} glyph_instances={} atlas_version={} atlas_bytes={}",
            ms(prewarm.elapsed),
            prewarm.frames,
            prewarm.glyph_instances,
            prewarm.atlas_version,
            prewarm.atlas_bytes
        );
    }

    let mut samples = Vec::with_capacity(options.frames);
    if let Some(uri) = &options.video_uri {
        run_video_driven(uri, &options, viewport, &mut engine, &mut samples)?;
    } else {
        run_synthetic(&options, viewport, &mut engine, &mut samples);
    }

    print_report(prepare_elapsed, &samples);
    Ok(())
}

fn run_synthetic(
    options: &PerfOptions,
    viewport: DanmakuViewport,
    engine: &mut DfmLayoutEngine,
    samples: &mut Vec<FrameSample>,
) {
    for frame_index in 0..options.frames {
        let media_time = Duration::from_secs_f64(frame_index as f64 / options.fps);
        samples.push(run_one_frame(
            engine,
            viewport,
            media_time,
            frame_index as u64 + 1,
            None,
        ));
    }
}

fn run_video_driven(
    uri: &str,
    options: &PerfOptions,
    viewport: DanmakuViewport,
    engine: &mut DfmLayoutEngine,
    samples: &mut Vec<FrameSample>,
) -> Result<(), String> {
    let mut playback = VideoPlaybackEngine::open(
        &MediaRequest::new(uri),
        PlaybackSessionConfig {
            video_decode: options.decode_preference,
            ..PlaybackSessionConfig::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let info = playback.info();
    println!(
        "video: duration={:?} params={:?} decoder={:?}",
        info.duration, info.video_params, info.video_decode_backend
    );
    playback.play();
    let started = Instant::now();
    let mut empty_ticks = 0u64;
    while samples.len() < options.frames {
        if started.elapsed() > Duration::from_secs(30) {
            return Err(format!(
                "timed out waiting for video frames: got {} / {}",
                samples.len(),
                options.frames
            ));
        }
        match playback.tick() {
            Ok(Some(frame)) => {
                let media_time = frame.pts.unwrap_or(frame.media_time);
                samples.push(run_one_frame(
                    engine,
                    viewport,
                    media_time,
                    samples.len() as u64 + 1,
                    Some(VideoSample {
                        width: frame.frame.width() as usize,
                        height: frame.frame.height() as usize,
                        is_hardware: frame.frame.is_videotoolbox(),
                        late_by: frame.late_by,
                    }),
                ));
            }
            Ok(None) => {
                empty_ticks += 1;
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    println!("video_ticks: empty={empty_ticks}");
    Ok(())
}

fn prewarm_atlas(
    engine: &mut DfmLayoutEngine,
    viewport: DanmakuViewport,
    options: &PerfOptions,
) -> PrewarmStats {
    if options.prewarm_frames == 0 {
        return PrewarmStats::default();
    }
    let started = Instant::now();
    let mut glyph_instances = 0usize;
    let mut atlas_version = 0u64;
    let mut atlas_bytes = 0usize;
    let denominator = options.prewarm_frames.saturating_sub(1).max(1) as f64;
    for index in 0..options.prewarm_frames {
        let ratio = index as f64 / denominator;
        let media_time = Duration::from_secs_f64(options.duration_seconds * ratio);
        let plan = engine.render_plan(media_time, viewport, index as u64 + 1);
        glyph_instances = glyph_instances.saturating_add(plan.items.len());
        if let Some(atlas) = plan.atlas.as_ref() {
            atlas_version = atlas.version;
            atlas_bytes = atlas.required_len().saturating_mul(2);
        }
    }
    PrewarmStats {
        elapsed: started.elapsed(),
        frames: options.prewarm_frames,
        glyph_instances,
        atlas_version,
        atlas_bytes,
    }
}

fn run_one_frame(
    engine: &mut DfmLayoutEngine,
    viewport: DanmakuViewport,
    media_time: Duration,
    generation: u64,
    video: Option<VideoSample>,
) -> FrameSample {
    let frame_started = Instant::now();
    let layout_started = Instant::now();
    let frame_layout = engine.frame_layout(media_time, viewport, generation);
    let layout_elapsed = layout_started.elapsed();

    let render_started = Instant::now();
    let plan = engine.render_plan(media_time, viewport, generation);
    let render_elapsed = render_started.elapsed();
    let atlas = plan.atlas.as_ref();
    let glyph_instances = plan.items.len();
    let visible_items = frame_layout.items.len();
    let shadow_draws = plan
        .items
        .iter()
        .filter(|item| item.shadow_rgba[3] > 0.0)
        .count();
    let outline_draws = plan
        .items
        .iter()
        .filter(|item| item.outline_rgba[3] > 0.0)
        .count();
    let fill_draws = glyph_instances;

    FrameSample {
        media_time,
        total_elapsed: frame_started.elapsed(),
        layout_elapsed,
        render_plan_elapsed: render_elapsed,
        visible_items,
        glyph_instances,
        current_metal_draws: shadow_draws + outline_draws + fill_draws,
        batched_draws: estimated_batched_draws(shadow_draws, outline_draws, fill_draws),
        atlas_version: atlas.map_or(0, |atlas| atlas.version),
        atlas_bytes: atlas.map_or(0, |atlas| atlas.required_len().saturating_mul(2)),
        video,
    }
}

fn estimated_batched_draws(shadow_draws: usize, outline_draws: usize, fill_draws: usize) -> usize {
    usize::from(shadow_draws > 0) + usize::from(outline_draws > 0) + usize::from(fill_draws > 0)
}

fn print_report(prepare_elapsed: Duration, samples: &[FrameSample]) {
    let mut layout = samples
        .iter()
        .map(|sample| sample.layout_elapsed)
        .collect::<Vec<_>>();
    let mut render_plan = samples
        .iter()
        .map(|sample| sample.render_plan_elapsed)
        .collect::<Vec<_>>();
    let mut total = samples
        .iter()
        .map(|sample| sample.total_elapsed)
        .collect::<Vec<_>>();
    let visible = samples
        .iter()
        .map(|sample| sample.visible_items)
        .collect::<Vec<_>>();
    let glyphs = samples
        .iter()
        .map(|sample| sample.glyph_instances)
        .collect::<Vec<_>>();
    let current_draws = samples
        .iter()
        .map(|sample| sample.current_metal_draws)
        .collect::<Vec<_>>();
    let batched_draws = samples
        .iter()
        .map(|sample| sample.batched_draws)
        .collect::<Vec<_>>();
    let atlas_changes = samples
        .windows(2)
        .filter(|pair| pair[0].atlas_version != pair[1].atlas_version)
        .count()
        + usize::from(
            samples
                .first()
                .is_some_and(|sample| sample.atlas_version > 0),
        );
    let max_atlas_bytes = samples
        .iter()
        .map(|sample| sample.atlas_bytes)
        .max()
        .unwrap_or(0);

    println!("\nsummary");
    println!("prepare_ms: {:.3}", ms(prepare_elapsed));
    print_duration_stats("frame_total_ms", &mut total);
    print_duration_stats("standalone_frame_layout_ms", &mut layout);
    print_duration_stats("render_plan_total_ms", &mut render_plan);
    print_usize_stats("visible_items", &visible);
    print_usize_stats("glyph_instances", &glyphs);
    print_usize_stats("current_metal_draws", &current_draws);
    print_usize_stats("batched_target_draws", &batched_draws);
    let current_avg = avg_usize(&current_draws);
    let batched_avg = avg_usize(&batched_draws).max(1.0);
    println!(
        "draw_call_reduction_target: {:.1}x fewer draws after batching",
        current_avg / batched_avg
    );
    println!("atlas_changes: {atlas_changes}");
    println!("atlas_bytes_max: {max_atlas_bytes}");

    if let Some(video) = samples.iter().find_map(|sample| sample.video) {
        let hardware_frames = samples
            .iter()
            .filter(|sample| sample.video.is_some_and(|video| video.is_hardware))
            .count();
        let max_late = samples
            .iter()
            .filter_map(|sample| sample.video.and_then(|video| video.late_by))
            .max()
            .unwrap_or(Duration::ZERO);
        println!(
            "video_frames: {} first={}x{} hw_frames={} max_late_ms={:.3}",
            samples
                .iter()
                .filter(|sample| sample.video.is_some())
                .count(),
            video.width,
            video.height,
            hardware_frames,
            ms(max_late)
        );
    }

    if let Some(worst) = samples
        .iter()
        .max_by_key(|sample| sample.render_plan_elapsed)
    {
        println!(
            "worst_render_plan_frame: t={:.3}s render_plan_total_ms={:.3} visible={} glyphs={} atlas_version={}",
            worst.media_time.as_secs_f64(),
            ms(worst.render_plan_elapsed),
            worst.visible_items,
            worst.glyph_instances,
            worst.atlas_version
        );
    }
}

fn print_duration_stats(label: &str, values: &mut [Duration]) {
    values.sort_unstable();
    println!(
        "{label}: avg={:.3} p50={:.3} p95={:.3} max={:.3}",
        ms(avg_duration(values)),
        ms(percentile_duration(values, 50)),
        ms(percentile_duration(values, 95)),
        ms(*values.last().unwrap_or(&Duration::ZERO))
    );
}

fn print_usize_stats(label: &str, values: &[usize]) {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let avg = avg_usize(&sorted);
    println!(
        "{label}: avg={avg:.1} p50={} p95={} max={}",
        percentile_usize(&sorted, 50),
        percentile_usize(&sorted, 95),
        sorted.last().copied().unwrap_or(0)
    );
}

fn avg_usize(values: &[usize]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<usize>() as f64 / values.len() as f64
    }
}

fn percentile_duration(values: &[Duration], percentile: usize) -> Duration {
    if values.is_empty() {
        return Duration::ZERO;
    }
    values[percentile_index(values.len(), percentile)]
}

fn percentile_usize(values: &[usize], percentile: usize) -> usize {
    if values.is_empty() {
        return 0;
    }
    values[percentile_index(values.len(), percentile)]
}

fn percentile_index(len: usize, percentile: usize) -> usize {
    let percentile = percentile.min(100);
    (((len - 1) * percentile) + 50) / 100
}

fn avg_duration(values: &[Duration]) -> Duration {
    if values.is_empty() {
        return Duration::ZERO;
    }
    let total_ns = values.iter().map(Duration::as_nanos).sum::<u128>();
    Duration::from_nanos((total_ns / values.len() as u128).min(u64::MAX as u128) as u64)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn load_or_generate_timeline(options: &PerfOptions) -> Result<DanmakuTimeline, String> {
    if let Some(path) = &options.danmaku_path {
        return DanmakuTimeline::from_file(path).map_err(|error| error.to_string());
    }
    generate_timeline(options)
}

fn generate_timeline(options: &PerfOptions) -> Result<DanmakuTimeline, String> {
    let mut items = Vec::with_capacity(options.item_count);
    let duration = options.duration_seconds.max(0.001);
    for index in 0..options.item_count {
        let time = (index as f64 / options.rate.max(0.001)).min(duration);
        let mode = mode_for(options.pattern, index);
        let color = color_for(index);
        let text = text_for(options.pattern, index);
        items.push(DanmakuItem {
            id: index as u64 + 1,
            pts: Duration::from_secs_f64(time),
            text,
            mode,
            font_size: 25.0,
            color,
            opacity: 1.0,
            is_self: index % 97 == 0,
        });
    }
    DanmakuTimeline::new(items).map_err(|error| error.to_string())
}

fn mode_for(pattern: PerfPattern, index: usize) -> DanmakuMode {
    match pattern {
        PerfPattern::Scroll => DanmakuMode::Scroll,
        PerfPattern::Fixed => {
            if index % 2 == 0 {
                DanmakuMode::Top
            } else {
                DanmakuMode::Bottom
            }
        }
        PerfPattern::Dense | PerfPattern::Mixed => match index % 20 {
            0 | 1 => DanmakuMode::Top,
            2 => DanmakuMode::Bottom,
            3 if matches!(pattern, PerfPattern::Mixed) => DanmakuMode::ScrollReverse,
            _ => DanmakuMode::Scroll,
        },
    }
}

fn text_for(pattern: PerfPattern, index: usize) -> String {
    const BASE: &[&str] = &[
        "这句好戳",
        "现在这条应该跟着画面走",
        "Kuroko 原生弹幕压测",
        "seek 后不能回跳",
        "DFM+ track collision",
        "莉可丽丝",
        "画面停弹幕也停",
        "glyph atlas reuse",
        "批量绘制前基准",
        "高密度滚动测试",
    ];
    match pattern {
        PerfPattern::Dense => format!("{} #{:05}", BASE[index % BASE.len()], index),
        _ => format!("{} {}", BASE[index % BASE.len()], suffix_for(index)),
    }
}

fn suffix_for(index: usize) -> &'static str {
    match index % 8 {
        0 => "草",
        1 => "2333",
        2 => "wwww",
        3 => "!!!",
        4 => "x5",
        5 => "弹幕雨",
        6 => "同步",
        _ => "GPU",
    }
}

fn color_for(index: usize) -> DanmakuColor {
    const COLORS: &[(u8, u8, u8)] = &[
        (255, 255, 255),
        (255, 218, 86),
        (101, 220, 255),
        (255, 112, 169),
        (122, 255, 159),
        (218, 189, 255),
    ];
    let (red, green, blue) = COLORS[index % COLORS.len()];
    DanmakuColor::rgb_u8(red, green, blue)
}

#[derive(Debug, Clone, PartialEq)]
struct PerfOptions {
    danmaku_path: Option<String>,
    video_uri: Option<String>,
    decode_preference: VideoDecodePreference,
    item_count: usize,
    rate: f64,
    duration_seconds: f64,
    frames: usize,
    prewarm_frames: usize,
    fps: f64,
    width: u32,
    height: u32,
    font_size: f32,
    display_area: f32,
    scroll_duration_seconds: f32,
    outline_width: f32,
    shadow_style: DanmakuShadowStyle,
    merge_duplicates: bool,
    allow_stacking: bool,
    pattern: PerfPattern,
}

impl Default for PerfOptions {
    fn default() -> Self {
        let duration_seconds = 120.0;
        let rate = 50.0;
        Self {
            danmaku_path: None,
            video_uri: None,
            decode_preference: VideoDecodePreference::default(),
            item_count: (duration_seconds * rate) as usize,
            rate,
            duration_seconds,
            frames: 600,
            prewarm_frames: 0,
            fps: 60.0,
            width: 1920,
            height: 1080,
            font_size: 30.0,
            display_area: 1.0,
            scroll_duration_seconds: 10.0,
            outline_width: 1.0,
            shadow_style: DanmakuShadowStyle::Strong,
            merge_duplicates: false,
            allow_stacking: false,
            pattern: PerfPattern::Mixed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PerfPattern {
    Mixed,
    Scroll,
    Fixed,
    Dense,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VideoSample {
    width: usize,
    height: usize,
    is_hardware: bool,
    late_by: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
struct FrameSample {
    media_time: Duration,
    total_elapsed: Duration,
    layout_elapsed: Duration,
    render_plan_elapsed: Duration,
    visible_items: usize,
    glyph_instances: usize,
    current_metal_draws: usize,
    batched_draws: usize,
    atlas_version: u64,
    atlas_bytes: usize,
    video: Option<VideoSample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PrewarmStats {
    elapsed: Duration,
    frames: usize,
    glyph_instances: usize,
    atlas_version: u64,
    atlas_bytes: usize,
}

fn parse_args(args: &[String]) -> Result<PerfOptions, String> {
    let mut options = PerfOptions::default();
    let mut explicit_items = false;
    let mut explicit_rate = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--danmaku" => options.danmaku_path = Some(next_string(args, &mut index, "--danmaku")?),
            "--video" => options.video_uri = Some(next_string(args, &mut index, "--video")?),
            "--software" => options.decode_preference = VideoDecodePreference::Software,
            "--videotoolbox" => options.decode_preference = VideoDecodePreference::VideoToolbox,
            "--items" => {
                options.item_count = next_parse(args, &mut index, "--items")?;
                explicit_items = true;
            }
            "--rate" => {
                options.rate = next_parse(args, &mut index, "--rate")?;
                explicit_rate = true;
            }
            "--duration" => options.duration_seconds = next_parse(args, &mut index, "--duration")?,
            "--frames" => options.frames = next_parse(args, &mut index, "--frames")?,
            "--prewarm-frames" => {
                options.prewarm_frames = next_parse(args, &mut index, "--prewarm-frames")?
            }
            "--fps" => options.fps = next_parse(args, &mut index, "--fps")?,
            "--size" => {
                let value = next_string(args, &mut index, "--size")?;
                let (width, height) = parse_size(&value)?;
                options.width = width;
                options.height = height;
            }
            "--font-size" => options.font_size = next_parse(args, &mut index, "--font-size")?,
            "--display-area" => {
                options.display_area = next_parse(args, &mut index, "--display-area")?
            }
            "--scroll-duration" => {
                options.scroll_duration_seconds = next_parse(args, &mut index, "--scroll-duration")?
            }
            "--outline" => options.outline_width = next_parse(args, &mut index, "--outline")?,
            "--no-shadow" => options.shadow_style = DanmakuShadowStyle::None,
            "--merge" => options.merge_duplicates = true,
            "--stacking" => options.allow_stacking = true,
            "--pattern" => {
                options.pattern = parse_pattern(&next_string(args, &mut index, "--pattern")?)?
            }
            value if value.starts_with("--") => return Err(format!("unknown option: {value}")),
            value => {
                if options.video_uri.replace(value.to_string()).is_some() {
                    return Err("video path was provided more than once".to_string());
                }
            }
        }
        index += 1;
    }
    validate_options(&mut options, explicit_items, explicit_rate)?;
    Ok(options)
}

fn validate_options(
    options: &mut PerfOptions,
    explicit_items: bool,
    explicit_rate: bool,
) -> Result<(), String> {
    if !options.duration_seconds.is_finite() || options.duration_seconds <= 0.0 {
        return Err("--duration must be positive and finite".to_string());
    }
    if !options.rate.is_finite() || options.rate <= 0.0 {
        return Err("--rate must be positive and finite".to_string());
    }
    if options.frames == 0 {
        return Err("--frames must be greater than zero".to_string());
    }
    if !options.fps.is_finite() || options.fps <= 0.0 {
        return Err("--fps must be positive and finite".to_string());
    }
    if options.width == 0 || options.height == 0 {
        return Err("--size dimensions must be greater than zero".to_string());
    }
    if !options.font_size.is_finite() || options.font_size <= 0.0 {
        return Err("--font-size must be positive and finite".to_string());
    }
    if !options.display_area.is_finite() || options.display_area <= 0.0 {
        return Err("--display-area must be positive and finite".to_string());
    }
    if !options.scroll_duration_seconds.is_finite() || options.scroll_duration_seconds <= 0.0 {
        return Err("--scroll-duration must be positive and finite".to_string());
    }
    if !options.outline_width.is_finite() || options.outline_width < 0.0 {
        return Err("--outline must be finite and non-negative".to_string());
    }
    if !explicit_items && explicit_rate {
        options.item_count = (options.duration_seconds * options.rate).round() as usize;
    } else if explicit_items && !explicit_rate {
        options.rate = options.item_count as f64 / options.duration_seconds;
    }
    if options.item_count == 0 {
        return Err("--items must be greater than zero".to_string());
    }
    Ok(())
}

fn next_string(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn next_parse<T>(args: &[String], index: &mut usize, flag: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    let value = next_string(args, index, flag)?;
    value
        .parse::<T>()
        .map_err(|_| format!("invalid {flag} value: {value}"))
}

fn parse_size(value: &str) -> Result<(u32, u32), String> {
    let Some((width, height)) = value.split_once('x').or_else(|| value.split_once('X')) else {
        return Err("--size must look like 1920x1080".to_string());
    };
    let width = width
        .parse::<u32>()
        .map_err(|_| format!("invalid width in --size: {value}"))?;
    let height = height
        .parse::<u32>()
        .map_err(|_| format!("invalid height in --size: {value}"))?;
    Ok((width, height))
}

fn parse_pattern(value: &str) -> Result<PerfPattern, String> {
    match value {
        "mixed" => Ok(PerfPattern::Mixed),
        "scroll" => Ok(PerfPattern::Scroll),
        "fixed" => Ok(PerfPattern::Fixed),
        "dense" => Ok(PerfPattern::Dense),
        _ => Err(format!("unknown pattern: {value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_derives_items_from_rate_and_duration() {
        let options = parse_args(&[
            "--rate".to_string(),
            "100".to_string(),
            "--duration".to_string(),
            "10".to_string(),
        ])
        .unwrap();

        assert_eq!(options.item_count, 1000);
        assert_eq!(options.rate, 100.0);
    }

    #[test]
    fn parse_args_accepts_size_and_pattern() {
        let options = parse_args(&[
            "--size".to_string(),
            "1280x720".to_string(),
            "--pattern".to_string(),
            "dense".to_string(),
        ])
        .unwrap();

        assert_eq!(options.width, 1280);
        assert_eq!(options.height, 720);
        assert_eq!(options.pattern, PerfPattern::Dense);
    }
}
