use std::cell::RefCell;
use std::env;
use std::ffi::c_void;
use std::fs;
use std::process;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use kuroko::danmaku::{DanmakuItem, DanmakuMode, DanmakuTimeline, parse_json_lines};
use kuroko::overlay::OverlayTimeline;
use kuroko::presenter::{PresenterConfig, PresenterRuntime, PresenterStats};
use kuroko::renderer::metal::{MetalOutputMode, MetalRendererConfig};
#[cfg(feature = "libass")]
use kuroko::subtitle::LibassRenderConfig;
use kuroko::subtitle::{SubtitleCue, SubtitleTimeline};
use kuroko::{MediaRequest, MetalSurfaceHandle, PlatformSurface};

static MEDIA_URI: OnceLock<String> = OnceLock::new();
static SUBTITLE_PATH: OnceLock<String> = OnceLock::new();
static DANMAKU_PATH: OnceLock<String> = OnceLock::new();
static SMOKE_SECONDS: OnceLock<f64> = OnceLock::new();
static EDR_HEADROOM: OnceLock<f32> = OnceLock::new();
static TELEMETRY_INTERVAL: OnceLock<f64> = OnceLock::new();

unsafe extern "C" {
    fn kuroko_demo_run_app();
}

thread_local! {
    static DEMO: RefCell<DemoState> = RefCell::new(DemoState::new().expect("create demo state"));
}

struct DemoState {
    presenter: PresenterRuntime,
    load_attempted: bool,
    overlay_logged: bool,
    telemetry: DemoTelemetry,
}

impl DemoState {
    fn new() -> kuroko::Result<Self> {
        Ok(Self {
            presenter: PresenterRuntime::new(PresenterConfig {
                renderer: demo_renderer_config(),
                overlay: demo_overlay_timeline(),
                ..PresenterConfig::default()
            })?,
            load_attempted: false,
            overlay_logged: false,
            telemetry: DemoTelemetry::new(),
        })
    }

    fn render(&mut self, time_seconds: f64) {
        if !self.load_attempted {
            self.load_attempted = true;
            if let Some(uri) = MEDIA_URI.get() {
                match self.presenter.open(MediaRequest::new(uri)) {
                    Ok(()) => {
                        if let Some(path) = SUBTITLE_PATH.get() {
                            match self.presenter.add_external_subtitle(path) {
                                Ok(track) => eprintln!(
                                    "Kuroko demo added external subtitle track #{}: {path}",
                                    track.id
                                ),
                                Err(error) => {
                                    eprintln!("Kuroko demo external subtitle add failed: {error}")
                                }
                            }
                        }
                        match self.presenter.play() {
                            Ok(()) => {
                                eprintln!("Kuroko demo opened media through presenter runtime")
                            }
                            Err(error) => eprintln!("Kuroko demo play failed: {error}"),
                        }
                    }
                    Err(error) => eprintln!("Kuroko demo video load failed: {error}"),
                }
            }
        }

        let tick_started = Instant::now();
        match self.presenter.render_tick(time_seconds) {
            Ok(stats) => {
                if !self.overlay_logged && stats.overlay_frames > 0 {
                    eprintln!("Kuroko demo overlay active through presenter runtime");
                    self.overlay_logged = true;
                }
                self.telemetry
                    .sample(time_seconds, tick_started.elapsed(), stats);
            }
            Err(error) => eprintln!("Kuroko demo render failed: {error}"),
        }
    }
}

fn demo_renderer_config() -> MetalRendererConfig {
    let Some(headroom) = EDR_HEADROOM.get().copied() else {
        return MetalRendererConfig::default();
    };
    MetalRendererConfig {
        output_mode: MetalOutputMode::apple_edr(headroom),
    }
}

fn demo_overlay_timeline() -> OverlayTimeline {
    let subtitles = SubtitleTimeline::new(vec![SubtitleCue {
        start: Duration::from_millis(500),
        end: Duration::from_secs(4),
        text: "Kuroko native overlay".to_string(),
    }]);
    let timeline = OverlayTimeline::default().with_subtitles(subtitles);
    #[cfg(feature = "danmaku-next2")]
    {
        let danmaku = configured_danmaku_timeline();
        let fallback = timeline.clone().with_danmaku(danmaku.clone());
        timeline
            .with_next2_danmaku(danmaku)
            .unwrap_or_else(|error| {
                eprintln!("Kuroko demo Next2 danmaku setup failed: {error}");
                fallback
            })
    }
    #[cfg(all(not(feature = "danmaku-next2"), feature = "libass"))]
    {
        let danmaku = configured_danmaku_timeline();
        let fallback = timeline.clone().with_danmaku(danmaku.clone());
        timeline
            .with_ass_danmaku(danmaku, LibassRenderConfig::default())
            .unwrap_or_else(|error| {
                eprintln!("Kuroko demo libass danmaku setup failed: {error}");
                fallback
            })
    }
    #[cfg(all(not(feature = "danmaku-next2"), not(feature = "libass")))]
    {
        timeline.with_danmaku(configured_danmaku_timeline())
    }
}

fn configured_danmaku_timeline() -> DanmakuTimeline {
    let Some(path) = DANMAKU_PATH.get() else {
        return demo_danmaku_timeline();
    };
    match load_danmaku_jsonl(path) {
        Ok(timeline) => timeline,
        Err(error) => {
            eprintln!("Kuroko demo danmaku load failed: {error}");
            demo_danmaku_timeline()
        }
    }
}

fn load_danmaku_jsonl(path: &str) -> Result<DanmakuTimeline, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let items = parse_json_lines(&text).map_err(|error| format!("{path}: {error}"))?;
    let count = items.len();
    let mut timeline = DanmakuTimeline::default();
    timeline.extend(items);
    eprintln!("Kuroko demo loaded {count} danmaku items from {path}");
    #[cfg(feature = "danmaku-next2")]
    eprintln!("Kuroko demo visible danmaku renderer: NipaPlay Next2/MSDF");
    #[cfg(all(not(feature = "danmaku-next2"), feature = "libass"))]
    eprintln!("Kuroko demo visible danmaku renderer: libass bridge");
    #[cfg(all(not(feature = "danmaku-next2"), not(feature = "libass")))]
    eprintln!("Kuroko demo visible danmaku renderer: native boxes only, text draw pending");
    Ok(timeline)
}

fn demo_danmaku_timeline() -> DanmakuTimeline {
    let mut danmaku = DanmakuTimeline::default();
    danmaku.push(DanmakuItem {
        id: 1,
        pts: Duration::from_secs(1),
        text: "Rust danmaku timeline".to_string(),
        mode: DanmakuMode::Scroll,
        font_size: 32.0,
        color_rgba: [1.0, 1.0, 1.0, 1.0],
    });
    danmaku
}

#[unsafe(no_mangle)]
pub extern "C" fn kuroko_demo_attach_layer(
    layer: *mut c_void,
    width: u32,
    height: u32,
    scale: f64,
) {
    let surface =
        PlatformSurface::Metal(MetalSurfaceHandle::new(layer as u64, width, height, scale));
    DEMO.with(|demo| {
        if let Err(error) = demo.borrow_mut().presenter.attach_surface(surface) {
            eprintln!("Kuroko demo attach failed: {error}");
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn kuroko_demo_resize_layer(width: u32, height: u32, scale: f64) {
    DEMO.with(|demo| {
        if let Err(error) = demo
            .borrow_mut()
            .presenter
            .resize_surface(width, height, scale)
        {
            eprintln!("Kuroko demo resize failed: {error}");
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn kuroko_demo_render_frame(time_seconds: f64) {
    DEMO.with(|demo| demo.borrow_mut().render(time_seconds));
}

#[unsafe(no_mangle)]
pub extern "C" fn kuroko_demo_smoke_seconds() -> f64 {
    SMOKE_SECONDS.get().copied().unwrap_or(0.0)
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let options = parse_args(&args).unwrap_or_else(|error| {
        eprintln!("{error}");
        eprintln!(
            "usage: cargo run -p macos_native_demo -- [--edr [HEADROOM]] [--smoke-seconds N] [--subtitle PATH] [--danmaku-jsonl PATH] [--generate-danmaku-jsonl PATH] [--danmaku-count N] [--danmaku-duration N] [--telemetry-interval N] [media-path-or-uri]"
        );
        process::exit(2);
    });
    let generated_danmaku_path = if let Some(path) = options.generate_danmaku_path.as_deref() {
        if let Err(error) = write_stress_danmaku_jsonl(
            path,
            options.danmaku_count,
            options.danmaku_duration_seconds,
        ) {
            eprintln!("{error}");
            process::exit(1);
        }
        Some(path.to_string())
    } else {
        None
    };
    let danmaku_path = options.danmaku_path.or(generated_danmaku_path);
    if let Some(path) = danmaku_path {
        DANMAKU_PATH.set(path).expect("danmaku path is set once");
    }
    if let Some(path) = options.subtitle_path {
        SUBTITLE_PATH.set(path).expect("subtitle path is set once");
    }

    if let Some(headroom) = options.edr_headroom {
        EDR_HEADROOM
            .set(headroom)
            .expect("EDR headroom is set once");
        eprintln!("Kuroko demo EDR mode: RGBA16Float headroom {headroom:.2}x");
    }
    if let Some(seconds) = options.smoke_seconds {
        SMOKE_SECONDS
            .set(seconds)
            .expect("smoke seconds is set once");
        eprintln!("Kuroko demo smoke mode: exit after {seconds:.2}s");
    }
    if let Some(interval) = options.telemetry_interval.or_else(|| {
        DANMAKU_PATH
            .get()
            .map(|_| DemoOptions::default_telemetry_interval())
    }) {
        TELEMETRY_INTERVAL
            .set(interval)
            .expect("telemetry interval is set once");
        eprintln!("Kuroko demo telemetry interval: {interval:.2}s");
    }
    if let Some(uri) = options.media_uri {
        MEDIA_URI.set(uri).expect("media URI is set once");
    }
    unsafe { kuroko_demo_run_app() };
}

#[derive(Debug, Clone, PartialEq)]
struct DemoOptions {
    media_uri: Option<String>,
    smoke_seconds: Option<f64>,
    edr_headroom: Option<f32>,
    subtitle_path: Option<String>,
    danmaku_path: Option<String>,
    generate_danmaku_path: Option<String>,
    danmaku_count: usize,
    danmaku_duration_seconds: f64,
    telemetry_interval: Option<f64>,
}

impl DemoOptions {
    fn default_danmaku_count() -> usize {
        80_000
    }

    fn default_danmaku_duration_seconds() -> f64 {
        120.0
    }

    fn default_telemetry_interval() -> f64 {
        1.0
    }
}

fn parse_args(args: &[String]) -> Result<DemoOptions, String> {
    let mut media_uri = None;
    let mut smoke_seconds = None;
    let mut edr_headroom = None;
    let mut subtitle_path = None;
    let mut danmaku_path = None;
    let mut generate_danmaku_path = None;
    let mut danmaku_count = DemoOptions::default_danmaku_count();
    let mut danmaku_duration_seconds = DemoOptions::default_danmaku_duration_seconds();
    let mut telemetry_interval = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--edr" => {
                let mut headroom = 4.0;
                if let Some(value) = args.get(index + 1) {
                    if !value.starts_with("--") && value.parse::<f32>().is_ok() {
                        index += 1;
                        headroom = args[index].parse::<f32>().map_err(|_| {
                            format!("invalid --edr headroom value: {}", args[index])
                        })?;
                    }
                }
                if !headroom.is_finite() || headroom < 1.0 {
                    return Err("--edr headroom must be finite and at least 1.0".to_string());
                }
                edr_headroom = Some(headroom);
            }
            "--smoke-seconds" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--smoke-seconds requires a numeric value".to_string());
                };
                let seconds = value
                    .parse::<f64>()
                    .map_err(|_| format!("invalid --smoke-seconds value: {value}"))?;
                if !seconds.is_finite() || seconds <= 0.0 {
                    return Err("--smoke-seconds must be a positive finite number".to_string());
                }
                smoke_seconds = Some(seconds);
            }
            "--subtitle" | "--ass-subtitle" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(format!("{} requires a path", args[index - 1]));
                };
                if subtitle_path.replace(value.to_string()).is_some() {
                    return Err("subtitle path was provided more than once".to_string());
                }
            }
            "--danmaku-jsonl" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--danmaku-jsonl requires a path".to_string());
                };
                if danmaku_path.replace(value.to_string()).is_some() {
                    return Err("danmaku path was provided more than once".to_string());
                }
            }
            "--generate-danmaku-jsonl" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--generate-danmaku-jsonl requires a path".to_string());
                };
                if generate_danmaku_path.replace(value.to_string()).is_some() {
                    return Err("generated danmaku path was provided more than once".to_string());
                }
            }
            "--danmaku-count" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--danmaku-count requires a value".to_string());
                };
                danmaku_count = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --danmaku-count value: {value}"))?;
                if danmaku_count == 0 {
                    return Err("--danmaku-count must be greater than zero".to_string());
                }
            }
            "--danmaku-duration" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--danmaku-duration requires a numeric value".to_string());
                };
                danmaku_duration_seconds = value
                    .parse::<f64>()
                    .map_err(|_| format!("invalid --danmaku-duration value: {value}"))?;
                if !danmaku_duration_seconds.is_finite() || danmaku_duration_seconds <= 0.0 {
                    return Err("--danmaku-duration must be a positive finite number".to_string());
                }
            }
            "--telemetry-interval" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--telemetry-interval requires a numeric value".to_string());
                };
                let interval = value
                    .parse::<f64>()
                    .map_err(|_| format!("invalid --telemetry-interval value: {value}"))?;
                if !interval.is_finite() || interval <= 0.0 {
                    return Err("--telemetry-interval must be a positive finite number".to_string());
                }
                telemetry_interval = Some(interval);
            }
            "--" => {
                index += 1;
                if index >= args.len() {
                    break;
                }
                if media_uri.replace(args[index..].join(" ")).is_some() {
                    return Err("media path was provided more than once".to_string());
                }
                break;
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown option: {value}"));
            }
            value => {
                if media_uri.replace(value.to_string()).is_some() {
                    return Err("media path was provided more than once".to_string());
                }
            }
        }
        index += 1;
    }
    Ok(DemoOptions {
        media_uri,
        smoke_seconds,
        edr_headroom,
        subtitle_path,
        danmaku_path,
        generate_danmaku_path,
        danmaku_count,
        danmaku_duration_seconds,
        telemetry_interval,
    })
}

fn write_stress_danmaku_jsonl(
    path: &str,
    count: usize,
    duration_seconds: f64,
) -> Result<(), String> {
    let text = generate_stress_danmaku_jsonl(count, duration_seconds);
    fs::write(path, text).map_err(|error| format!("write danmaku stress file failed: {error}"))?;
    eprintln!(
        "Kuroko demo wrote {count} stress danmaku items to {path} over {duration_seconds:.2}s"
    );
    Ok(())
}

fn generate_stress_danmaku_jsonl(count: usize, duration_seconds: f64) -> String {
    let mut out = String::with_capacity(count.saturating_mul(112));
    for index in 0..count {
        let time = stress_time(index, count, duration_seconds);
        let mode = stress_mode(index);
        let font_size = stress_font_size(index);
        let color = stress_color(index);
        let text = stress_text(index);
        out.push_str(&format!(
            r#"{{"id":{},"time":{:.3},"mode":"{}","font_size":{:.1},"color":{},"text":"{}"}}"#,
            index + 1,
            time,
            mode,
            font_size,
            color,
            text
        ));
        out.push('\n');
    }
    out
}

fn stress_time(index: usize, count: usize, duration_seconds: f64) -> f64 {
    let base = index as f64 / count.max(1) as f64 * duration_seconds;
    let burst_phase = (index % 240) as f64;
    let burst = if burst_phase < 120.0 {
        burst_phase * 0.002
    } else {
        (burst_phase - 120.0) * 0.018
    };
    (base + burst).min(duration_seconds)
}

fn stress_mode(index: usize) -> &'static str {
    if index % 37 == 0 {
        "top"
    } else if index % 41 == 0 {
        "bottom"
    } else {
        "scroll"
    }
}

fn stress_font_size(index: usize) -> f32 {
    match index % 9 {
        0 => 20.0,
        1 => 22.0,
        2 => 25.0,
        3 => 28.0,
        4 => 32.0,
        5 => 36.0,
        6 => 42.0,
        7 => 48.0,
        _ => 56.0,
    }
}

fn stress_color(index: usize) -> u32 {
    const COLORS: &[u32] = &[
        0xffffff, 0xffe066, 0xff6b6b, 0x66d9ef, 0x95e06c, 0xc792ea, 0xf78c6c, 0x82aaff,
    ];
    COLORS[index % COLORS.len()]
}

fn stress_text(index: usize) -> String {
    const PATTERNS: &[&str] = &[
        "Kuroko native stress",
        "Lycoris SDR sample",
        "danmaku renderer pressure",
        "overlay telemetry",
        "弹幕压力测试",
        "字幕同层合成",
        "WWW",
        "long long comment crossing the whole frame",
        "seek safe timeline clock",
        "glyph atlas future path",
    ];
    format!("{} #{index}", PATTERNS[index % PATTERNS.len()])
}

struct DemoTelemetry {
    interval: Option<Duration>,
    last_log: Instant,
    last_stats: PresenterStats,
    ticks: u64,
    outer_tick_ns_total: u64,
    outer_tick_ns_max: u64,
}

impl DemoTelemetry {
    fn new() -> Self {
        Self {
            interval: TELEMETRY_INTERVAL
                .get()
                .copied()
                .map(Duration::from_secs_f64),
            last_log: Instant::now(),
            last_stats: PresenterStats::default(),
            ticks: 0,
            outer_tick_ns_total: 0,
            outer_tick_ns_max: 0,
        }
    }

    fn sample(&mut self, app_time_seconds: f64, outer_tick: Duration, stats: PresenterStats) {
        let Some(interval) = self.interval else {
            return;
        };
        let outer_tick_ns = duration_ns(outer_tick);
        self.ticks += 1;
        self.outer_tick_ns_total = self.outer_tick_ns_total.saturating_add(outer_tick_ns);
        self.outer_tick_ns_max = self.outer_tick_ns_max.max(outer_tick_ns);
        if self.last_log.elapsed() < interval {
            return;
        }

        let overlay_builds = stats
            .overlay_builds
            .saturating_sub(self.last_stats.overlay_builds);
        let overlay_build_ns = stats
            .overlay_build_ns_total
            .saturating_sub(self.last_stats.overlay_build_ns_total);
        let tick_ns = stats
            .render_tick_ns_total
            .saturating_sub(self.last_stats.render_tick_ns_total);
        let render_ticks = stats
            .render_ticks
            .saturating_sub(self.last_stats.render_ticks)
            .max(1);
        let danmaku_boxes = stats
            .total_overlay_danmaku_boxes
            .saturating_sub(self.last_stats.total_overlay_danmaku_boxes);
        let decoded = stats
            .decoded_video_frames
            .saturating_sub(self.last_stats.decoded_video_frames);
        let rendered = stats
            .rendered_video_frames
            .saturating_sub(self.last_stats.rendered_video_frames);
        let audio = stats
            .pushed_audio_frames
            .saturating_sub(self.last_stats.pushed_audio_frames);
        let overlay_frames = stats
            .overlay_frames
            .saturating_sub(self.last_stats.overlay_frames);
        let outer_avg = self.outer_tick_ns_total / self.ticks.max(1);
        let overlay_avg = if overlay_builds == 0 {
            0
        } else {
            overlay_build_ns / overlay_builds
        };
        let boxes_avg = if overlay_builds == 0 {
            0
        } else {
            danmaku_boxes / overlay_builds
        };

        eprintln!(
            "Kuroko telemetry app={app_time_seconds:.2}s ticks={} outer_avg={:.3}ms outer_max={:.3}ms presenter_avg={:.3}ms overlay_avg={:.3}ms overlay_max={:.3}ms decoded+{} rendered+{} audio+{} overlay+{} boxes_last={} boxes_avg={} boxes_max={} alpha_last={} rgba_last={} failures i/r/a={}/{}/{}",
            self.ticks,
            ns_to_ms(outer_avg),
            ns_to_ms(self.outer_tick_ns_max),
            ns_to_ms(tick_ns / render_ticks),
            ns_to_ms(overlay_avg),
            ns_to_ms(stats.overlay_build_ns_max),
            decoded,
            rendered,
            audio,
            overlay_frames,
            stats.last_overlay_danmaku_boxes,
            boxes_avg,
            stats.max_overlay_danmaku_boxes,
            stats.last_overlay_alpha_planes,
            stats.last_overlay_subtitle_planes,
            stats.import_failures,
            stats.render_failures,
            stats.audio_failures,
        );

        self.last_log = Instant::now();
        self.last_stats = stats;
        self.ticks = 0;
        self.outer_tick_ns_total = 0;
        self.outer_tick_ns_max = 0;
    }
}

fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

fn ns_to_ms(ns: u64) -> f64 {
    ns as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_media_and_smoke_seconds() {
        let args = vec![
            "--smoke-seconds".to_string(),
            "1.5".to_string(),
            "/tmp/movie.mp4".to_string(),
        ];

        let options = parse_args(&args).unwrap();

        assert_eq!(options.media_uri.as_deref(), Some("/tmp/movie.mp4"));
        assert_eq!(options.smoke_seconds, Some(1.5));
        assert_eq!(options.edr_headroom, None);
        assert_eq!(options.subtitle_path, None);
    }

    #[test]
    fn parse_args_accepts_edr_with_default_headroom() {
        let args = vec!["--edr".to_string(), "/tmp/movie.mp4".to_string()];

        let options = parse_args(&args).unwrap();

        assert_eq!(options.edr_headroom, Some(4.0));
        assert_eq!(options.media_uri.as_deref(), Some("/tmp/movie.mp4"));
    }

    #[test]
    fn parse_args_accepts_edr_with_explicit_headroom() {
        let args = vec!["--edr".to_string(), "2.5".to_string()];

        let options = parse_args(&args).unwrap();

        assert_eq!(options.edr_headroom, Some(2.5));
    }

    #[test]
    fn parse_args_rejects_invalid_edr_headroom() {
        let args = vec!["--edr".to_string(), "0".to_string()];

        let error = parse_args(&args).unwrap_err();

        assert!(error.contains("headroom"));
    }

    #[test]
    fn parse_args_rejects_non_positive_smoke_seconds() {
        let args = vec!["--smoke-seconds".to_string(), "0".to_string()];

        let error = parse_args(&args).unwrap_err();

        assert!(error.contains("positive"));
    }

    #[test]
    fn parse_args_accepts_subtitle_path() {
        let args = vec![
            "--subtitle".to_string(),
            "/tmp/subs.srt".to_string(),
            "/tmp/movie.mp4".to_string(),
        ];

        let options = parse_args(&args).unwrap();

        assert_eq!(options.subtitle_path.as_deref(), Some("/tmp/subs.srt"));
        assert_eq!(options.media_uri.as_deref(), Some("/tmp/movie.mp4"));
    }

    #[test]
    fn parse_args_keeps_ass_subtitle_alias() {
        let args = vec![
            "--ass-subtitle".to_string(),
            "/tmp/subs.ass".to_string(),
            "/tmp/movie.mp4".to_string(),
        ];

        let options = parse_args(&args).unwrap();

        assert_eq!(options.subtitle_path.as_deref(), Some("/tmp/subs.ass"));
        assert_eq!(options.media_uri.as_deref(), Some("/tmp/movie.mp4"));
    }

    #[test]
    fn parse_args_accepts_danmaku_stress_options() {
        let args = vec![
            "--generate-danmaku-jsonl".to_string(),
            "/tmp/stress.jsonl".to_string(),
            "--danmaku-count".to_string(),
            "1234".to_string(),
            "--danmaku-duration".to_string(),
            "90".to_string(),
            "--telemetry-interval".to_string(),
            "0.5".to_string(),
            "/tmp/movie.mp4".to_string(),
        ];

        let options = parse_args(&args).unwrap();

        assert_eq!(
            options.generate_danmaku_path.as_deref(),
            Some("/tmp/stress.jsonl")
        );
        assert_eq!(options.danmaku_count, 1234);
        assert_eq!(options.danmaku_duration_seconds, 90.0);
        assert_eq!(options.telemetry_interval, Some(0.5));
        assert_eq!(options.media_uri.as_deref(), Some("/tmp/movie.mp4"));
    }

    #[test]
    fn generated_stress_danmaku_is_jsonl_parseable() {
        let jsonl = generate_stress_danmaku_jsonl(32, 10.0);

        let items = parse_json_lines(&jsonl).unwrap();

        assert_eq!(items.len(), 32);
        assert!(items.iter().any(|item| item.mode == DanmakuMode::Top));
        assert!(
            items
                .iter()
                .any(|item| item.color_rgba != [1.0, 1.0, 1.0, 1.0])
        );
    }
}
