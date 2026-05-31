use std::env;
use std::process;
use std::time::Duration;

use kuroko::audio::{AudioOutputBackend, AudioRingBufferConfig, BufferedAudioOutput};
use kuroko::{MediaRequest, Player, PlayerConfig, PlayerEvent, PlayerState};

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!("usage: cargo run -p player_tick -- <media-path-or-uri> [frame-count]");
        process::exit(2);
    };
    let frame_limit = args
        .next()
        .map(|value| {
            value
                .parse::<usize>()
                .unwrap_or_else(|_| usage_error("frame-count must be a positive integer"))
        })
        .unwrap_or(8);

    let player = Player::new(PlayerConfig::default());
    let events = player.subscribe();
    let frames = player.subscribe_video_frames();
    let audio = player.subscribe_audio_frames();
    let mut audio_output = BufferedAudioOutput::new(AudioRingBufferConfig::default());
    player.open(MediaRequest::new(uri)).unwrap_or_else(|error| {
        eprintln!("player open failed: {error}");
        process::exit(1);
    });
    drain_open_events(&events);
    player.play().unwrap_or_else(|error| {
        eprintln!("player play failed: {error}");
        process::exit(1);
    });

    let mut presented = 0usize;
    let mut audio_printed = 0usize;
    while presented < frame_limit {
        while let Ok(pcm) = audio.try_recv() {
            if audio_printed < frame_limit {
                let peak = pcm
                    .frame
                    .samples
                    .iter()
                    .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
                println!(
                    "  player audio {:04} pts={:?} {}Hz/{}ch frames={} peak={:.3}",
                    audio_printed,
                    pcm.frame.pts,
                    pcm.frame.format.sample_rate,
                    pcm.frame.format.channels,
                    pcm.frame.frames,
                    peak,
                );
                audio_printed += 1;
            }
            audio_output.push(pcm.frame).unwrap_or_else(|error| {
                eprintln!("audio output push failed: {error}");
                process::exit(1);
            });
        }
        simulate_audio_callback(&mut audio_output);
        match frames.recv_timeout(Duration::from_secs(3)) {
            Ok(frame) => {
                println!(
                    "  player frame {:04} pts={:?} media={:?} late={:?} {}x{} pix_fmt={} hw={}",
                    presented,
                    frame.pts,
                    frame.media_time,
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
            Err(error) => {
                eprintln!("timed out waiting for player frame: {error}");
                process::exit(1);
            }
        }
    }

    player.close().unwrap_or_else(|error| {
        eprintln!("player close failed: {error}");
        process::exit(1);
    });
}

fn simulate_audio_callback(output: &mut BufferedAudioOutput) {
    if output.buffer().format().is_none() {
        return;
    }
    let channels = output.buffer().format().expect("format exists").channels as usize;
    let mut scratch = vec![0.0f32; 256 * channels];
    let result = output
        .read_interleaved(&mut scratch)
        .unwrap_or_else(|error| {
            eprintln!("audio output read failed: {error}");
            process::exit(1);
        });
    let stats = output.stats();
    if result.frames > 0 || result.underflow_frames > 0 {
        println!(
            "  audio sink read={} underflow={} queued={} dropped={}",
            result.frames, result.underflow_frames, stats.queued_frames, stats.dropped_frames,
        );
    }
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

fn usage_error(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2);
}
