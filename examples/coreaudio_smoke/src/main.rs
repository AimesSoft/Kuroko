use std::env;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use erika::apple::coreaudio::{CoreAudioOutput, CoreAudioOutputConfig};
use erika::{MediaRequest, Player, PlayerConfig, PlayerEvent, PlayerState};

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!("usage: cargo run -p coreaudio_smoke -- <media-path-or-uri> [seconds]");
        process::exit(2);
    };
    let seconds = args
        .next()
        .map(|value| {
            value
                .parse::<f64>()
                .unwrap_or_else(|_| usage_error("seconds must be a number"))
        })
        .unwrap_or(2.0);

    let player = Player::new(PlayerConfig::default());
    let events = player.subscribe();
    let video = player.subscribe_video_frames();
    let audio = player.subscribe_audio_frames();
    player.open(MediaRequest::new(uri)).unwrap_or_else(|error| {
        eprintln!("player open failed: {error}");
        process::exit(1);
    });
    drain_open_events(&events);

    let mut output = CoreAudioOutput::new(CoreAudioOutputConfig::default());
    player.play().unwrap_or_else(|error| {
        eprintln!("player play failed: {error}");
        process::exit(1);
    });

    let started = Instant::now();
    let mut configured = false;
    let mut pushed = 0usize;
    let mut video_frames = 0usize;
    while started.elapsed() < Duration::from_secs_f64(seconds.max(0.1)) {
        drain_runtime_events(&events);
        while let Ok(frame) = video.try_recv() {
            video_frames += 1;
            if video_frames <= 3 {
                println!(
                    "video frame {} pts={:?} media={:?} late={:?}",
                    video_frames, frame.pts, frame.media_time, frame.late_by
                );
            }
        }
        pushed += pump_audio(&audio, &mut output, &mut configured);
        thread::sleep(Duration::from_millis(2));
    }

    let wait_started = Instant::now();
    while configured
        && output
            .stats()
            .map(|stats| stats.read_frames == 0)
            .unwrap_or(false)
        && wait_started.elapsed() < Duration::from_secs(3)
    {
        drain_runtime_events(&events);
        pushed += pump_audio(&audio, &mut output, &mut configured);
        thread::sleep(Duration::from_millis(10));
    }

    let stats = output.stats().unwrap_or_default();
    let _ = output.stop();
    let _ = player.close();
    println!(
        "CoreAudio smoke done: pushed_blocks={} video_frames={} queued_frames={} read_frames={} underflow_frames={} dropped_frames={}",
        pushed,
        video_frames,
        stats.queued_frames,
        stats.read_frames,
        stats.underflow_frames,
        stats.dropped_frames,
    );
    if pushed < 3 {
        eprintln!("CoreAudio smoke pushed too few PCM blocks: {pushed}");
        process::exit(1);
    }
    if !configured {
        eprintln!("CoreAudio smoke never received an audio format to configure output");
        process::exit(1);
    }
    if stats.read_frames == 0 {
        eprintln!("CoreAudio smoke did not observe render-callback reads");
        process::exit(1);
    }
}

fn pump_audio(
    audio: &crossbeam_channel::Receiver<erika::PlayerAudioFrame>,
    output: &mut CoreAudioOutput,
    configured: &mut bool,
) -> usize {
    let mut pushed = 0usize;
    while let Ok(frame) = audio.try_recv() {
        if !*configured {
            output
                .configure(frame.frame.format)
                .unwrap_or_else(|error| {
                    eprintln!("CoreAudio configure failed: {error}");
                    process::exit(1);
                });
            output.start().unwrap_or_else(|error| {
                eprintln!("CoreAudio start failed: {error}");
                process::exit(1);
            });
            *configured = true;
        }
        output.push(frame.frame).unwrap_or_else(|error| {
            eprintln!("CoreAudio push failed: {error}");
            process::exit(1);
        });
        pushed += 1;
    }
    pushed
}

fn drain_open_events(events: &crossbeam_channel::Receiver<PlayerEvent>) {
    loop {
        match events.recv_timeout(Duration::from_secs(3)) {
            Ok(PlayerEvent::DurationChanged(duration)) => println!("duration: {duration:?}"),
            Ok(PlayerEvent::TracksChanged(tracks)) => println!("tracks: {}", tracks.len()),
            Ok(PlayerEvent::VideoParamsChanged(params)) => println!("video: {params:?}"),
            Ok(PlayerEvent::StateChanged(PlayerState::Ready)) => break,
            Ok(PlayerEvent::Error(error)) => {
                eprintln!("player error: {error}");
                process::exit(1);
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("timed out waiting for open events: {error}");
                process::exit(1);
            }
        }
    }
}

fn drain_runtime_events(events: &crossbeam_channel::Receiver<PlayerEvent>) {
    while let Ok(event) = events.try_recv() {
        match event {
            PlayerEvent::Error(error) => {
                eprintln!("player error: {error}");
                process::exit(1);
            }
            PlayerEvent::StateChanged(PlayerState::Stopped) => return,
            _ => {}
        }
    }
}

fn usage_error(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2);
}
