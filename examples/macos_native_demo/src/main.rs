use std::cell::RefCell;
use std::env;
use std::ffi::c_void;
use std::process;
use std::sync::OnceLock;
use std::time::Duration;

use kuroko::danmaku::{DanmakuItem, DanmakuMode, DanmakuTimeline};
use kuroko::overlay::OverlayTimeline;
use kuroko::presenter::{PresenterConfig, PresenterRuntime};
use kuroko::renderer::metal::{MetalOutputMode, MetalRendererConfig};
use kuroko::subtitle::{SubtitleCue, SubtitleTimeline};
use kuroko::{MediaRequest, MetalSurfaceHandle, PlatformSurface};

static MEDIA_URI: OnceLock<String> = OnceLock::new();
static SUBTITLE_PATH: OnceLock<String> = OnceLock::new();
static SMOKE_SECONDS: OnceLock<f64> = OnceLock::new();
static EDR_HEADROOM: OnceLock<f32> = OnceLock::new();

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

        match self.presenter.render_tick(time_seconds) {
            Ok(stats) => {
                if !self.overlay_logged && stats.overlay_frames > 0 {
                    eprintln!("Kuroko demo overlay active through presenter runtime");
                    self.overlay_logged = true;
                }
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
    OverlayTimeline::default()
        .with_subtitles(subtitles)
        .with_danmaku(demo_danmaku_timeline())
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
            "usage: cargo run -p macos_native_demo -- [--edr [HEADROOM]] [--smoke-seconds N] [--subtitle PATH] [--ass-subtitle PATH] [media-path-or-uri]"
        );
        process::exit(2);
    });
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
}

fn parse_args(args: &[String]) -> Result<DemoOptions, String> {
    let mut media_uri = None;
    let mut smoke_seconds = None;
    let mut edr_headroom = None;
    let mut subtitle_path = None;
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
    })
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
}
