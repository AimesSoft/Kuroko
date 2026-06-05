use std::env;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use kuroko::MediaRequest;
use kuroko::danmaku::{DanmakuLayoutConfig, DanmakuTimeline, DanmakuViewport, DfmLayoutEngine};
use kuroko::playback::{PlaybackSessionConfig, VideoDecodePreference, VideoPlaybackEngine};

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!(
            "usage: cargo run -p danmaku_sync_smoke -- <media-path-or-uri> <danmaku-json-jsonl-or-xml>"
        );
        process::exit(2);
    };
    let Some(danmaku_path) = args.next() else {
        eprintln!("missing danmaku file path");
        process::exit(2);
    };

    match run(&uri, &danmaku_path) {
        Ok(()) => println!("danmaku sync smoke OK"),
        Err(error) => {
            eprintln!("danmaku sync smoke failed: {error}");
            process::exit(1);
        }
    }
}

fn run(uri: &str, danmaku_path: &str) -> Result<(), String> {
    let request = MediaRequest::new(uri);
    let mut engine = VideoPlaybackEngine::open(
        &request,
        PlaybackSessionConfig {
            video_decode: VideoDecodePreference::Software,
            ..PlaybackSessionConfig::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let timeline = DanmakuTimeline::from_file(danmaku_path).map_err(|error| error.to_string())?;
    let mut layout = DfmLayoutEngine::new(timeline, DanmakuLayoutConfig::default());
    let mut generation = 1u64;

    engine.play();
    let first = next_video_plan(&mut engine, &mut layout, generation)?;
    println!(
        "first: media={:?} generation={} items={}",
        first.media_time,
        first.generation,
        first.items.len()
    );

    engine
        .seek(Duration::from_millis(1600))
        .map_err(|error| error.to_string())?;
    generation += 1;
    let after_seek = next_video_plan(&mut engine, &mut layout, generation)?;
    if after_seek.generation == first.generation {
        return Err("seek did not advance danmaku generation".to_string());
    }
    if after_seek.media_time < Duration::from_millis(1200) {
        return Err(format!(
            "seek returned a stale media time: {:?}",
            after_seek.media_time
        ));
    }

    engine.pause();
    let paused_a = layout.render_plan(
        after_seek.media_time,
        after_seek.viewport,
        after_seek.generation,
    );
    thread::sleep(Duration::from_millis(80));
    let paused_b = layout.render_plan(
        after_seek.media_time,
        after_seek.viewport,
        after_seek.generation,
    );
    if paused_a.items.first().map(|item| item.rect) != paused_b.items.first().map(|item| item.rect)
    {
        return Err("pause allowed danmaku to advance without media time changing".to_string());
    }

    engine.set_playback_rate(1.75);
    engine.play();
    let after_rate = next_video_plan(&mut engine, &mut layout, generation)?;
    if after_rate.media_time < after_seek.media_time {
        return Err("rate change moved danmaku backward outside media timeline".to_string());
    }

    let switched = DanmakuTimeline::parse_auto(
        r##"{"comments":[{"id":9001,"time":1.6,"content":"switched track","type":"scroll","color":"#ffffff"}]}"##,
    )
    .map_err(|error| error.to_string())?;
    layout.set_timeline(switched);
    generation += 1;
    let switched_plan = layout.render_plan(after_rate.media_time, after_rate.viewport, generation);
    if switched_plan.generation != generation {
        return Err("track switch plan did not use the new generation".to_string());
    }
    if after_rate.media_time >= Duration::from_millis(1600)
        && switched_plan
            .items
            .first()
            .is_some_and(|item| item.item_id != 9001)
    {
        return Err("track switch did not replace visible danmaku items".to_string());
    }

    Ok(())
}

#[derive(Debug)]
struct FramePlan {
    media_time: Duration,
    generation: u64,
    viewport: DanmakuViewport,
    items: Vec<kuroko::danmaku::DanmakuGlyphInstance>,
}

fn next_video_plan(
    engine: &mut VideoPlaybackEngine,
    layout: &mut DfmLayoutEngine,
    generation: u64,
) -> Result<FramePlan, String> {
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(5) {
            return Err("timed out waiting for video frame".to_string());
        }
        match engine.tick() {
            Ok(Some(frame)) => {
                let media_time = frame.pts.unwrap_or(frame.media_time);
                let viewport = DanmakuViewport::new(frame.frame.width(), frame.frame.height());
                let plan = layout.render_plan(media_time, viewport, generation);
                if plan.media_time != media_time || plan.generation != generation {
                    return Err(
                        "danmaku plan did not match frame media time/generation".to_string()
                    );
                }
                return Ok(FramePlan {
                    media_time,
                    generation: plan.generation,
                    viewport,
                    items: plan.items,
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(error) => return Err(error.to_string()),
        }
    }
}
