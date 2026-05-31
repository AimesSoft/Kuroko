use std::env;
use std::process;

use kuroko::MediaRequest;
use kuroko::playback::{PlaybackSession, PlaybackSessionConfig};

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!(
            "usage: cargo run -p playback_audio_tick -- <media-path-or-uri> [pcm-frame-count]"
        );
        process::exit(2);
    };
    let frame_limit = args
        .next()
        .map(|value| {
            value
                .parse::<usize>()
                .unwrap_or_else(|_| usage_error("pcm-frame-count must be a positive integer"))
        })
        .unwrap_or(8);

    let request = MediaRequest::new(uri);
    let mut session = PlaybackSession::open(&request, PlaybackSessionConfig::default())
        .unwrap_or_else(|error| {
            eprintln!("playback open failed: {error}");
            process::exit(1);
        });
    let info = session.info();
    println!("Kuroko playback audio tick");
    println!("uri: {}", info.uri);
    println!("duration: {:?}", info.duration);
    println!("selected audio: {:?}", info.selected_audio_track);
    println!("audio output: {:?}", info.audio_output);

    for index in 0..frame_limit {
        match session.next_audio_frame() {
            Ok(Some(pcm)) => {
                let peak = pcm
                    .samples
                    .iter()
                    .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
                println!(
                    "  pcm {:04} pts={:?} {}Hz/{}ch frames={} samples={} peak={:.3}",
                    index,
                    pcm.pts,
                    pcm.format.sample_rate,
                    pcm.format.channels,
                    pcm.frames,
                    pcm.samples.len(),
                    peak,
                );
            }
            Ok(None) => {
                eprintln!("audio ended before requested frame count");
                process::exit(1);
            }
            Err(error) => {
                eprintln!("playback audio tick failed: {error}");
                process::exit(1);
            }
        }
    }
}

fn usage_error(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2);
}
