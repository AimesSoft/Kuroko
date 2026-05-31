use std::env;
use std::process;

use kuroko::ffmpeg::{MediaProbe, probe_uri, version};

fn main() {
    let Some(uri) = env::args().nth(1) else {
        eprintln!("usage: cargo run -p ffmpeg_probe -- <media-path-or-uri>");
        process::exit(2);
    };

    match probe_uri(&uri) {
        Ok(probe) => print_probe(&probe),
        Err(error) => {
            eprintln!("ffmpeg probe failed: {error}");
            process::exit(1);
        }
    }
}

fn print_probe(probe: &MediaProbe) {
    println!("Kuroko FFmpeg probe");
    println!("ffmpeg: {}", version());
    println!("uri: {}", probe.uri);
    match probe.duration {
        Some(duration) => println!("duration: {:.3}s", duration.as_secs_f64()),
        None => println!("duration: unknown"),
    }

    println!("tracks: {}", probe.tracks.len());
    for track in &probe.tracks {
        println!(
            "  #{} {:?} codec={} language={} title={}",
            track.id,
            track.kind,
            track.codec.as_deref().unwrap_or("unknown"),
            track.language.as_deref().unwrap_or("-"),
            track.title.as_deref().unwrap_or("-")
        );
    }

    for video in &probe.video {
        println!(
            "video #{}: {}x{} codec={} pix_fmt={} primaries={:?} transfer={:?} profile={} level={}",
            video.track_id,
            video.params.width,
            video.params.height,
            video.codec.as_deref().unwrap_or("unknown"),
            video.pixel_format.as_deref().unwrap_or("unknown"),
            video.params.primaries,
            video.params.transfer,
            video.profile.as_deref().unwrap_or("unknown"),
            video
                .level
                .map_or("unknown".to_string(), |level| level.to_string())
        );
    }

    for audio in &probe.audio {
        println!(
            "audio #{}: codec={} sample_rate={} channels={} sample_fmt={}",
            audio.track_id,
            audio.codec.as_deref().unwrap_or("unknown"),
            audio.sample_rate,
            audio.channels,
            audio.sample_format.as_deref().unwrap_or("unknown"),
        );
    }
}
