use std::env;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use erika::MediaRequest;
use erika::playback::{PlaybackSessionConfig, VideoPlaybackEngine};

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!("usage: cargo run -p playback_tick -- <media-path-or-uri> [frame-count]");
        process::exit(2);
    };
    let frame_limit = args
        .next()
        .map(|value| {
            value
                .parse::<usize>()
                .unwrap_or_else(|_| usage_error("frame-count must be a positive integer"))
        })
        .unwrap_or(16);

    let request = MediaRequest::new(uri);
    let mut engine = VideoPlaybackEngine::open(&request, PlaybackSessionConfig::default())
        .unwrap_or_else(|error| {
            eprintln!("playback open failed: {error}");
            process::exit(1);
        });

    let info = engine.info();
    println!("Erika playback tick");
    println!("uri: {}", info.uri);
    println!("duration: {:?}", info.duration);
    println!("video: {:?}", info.video_params);
    println!("decoder: {:?}", info.video_decode_backend);

    engine.play();
    let started = Instant::now();
    let mut presented = 0usize;
    while presented < frame_limit {
        match engine.tick() {
            Ok(Some(frame)) => {
                println!(
                    "  frame {:04} pts={:?} media={:?} wall={:?} late={:?} {}x{} pix_fmt={} hw={}",
                    presented,
                    frame.pts,
                    frame.media_time,
                    started.elapsed(),
                    frame.late_by,
                    frame.frame.width(),
                    frame.frame.height(),
                    frame
                        .frame
                        .pixel_format()
                        .unwrap_or_else(|| format!("raw:{}", frame.frame.raw_pixel_format())),
                    frame.frame.is_videotoolbox(),
                );
                presented += 1;
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(error) => {
                eprintln!("playback tick failed: {error}");
                process::exit(1);
            }
        }
    }
}

fn usage_error(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2);
}
