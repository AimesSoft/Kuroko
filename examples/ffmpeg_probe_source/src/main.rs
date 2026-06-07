use std::env;
use std::process;

use erika::MediaSourceHint;
use erika::ffmpeg::{Demuxer, MediaProbe, version};
use erika::source::source_from_uri_with_hint;

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!(
            "usage: cargo run -p ffmpeg_probe_source -- <media-path-or-uri> [auto|file|http]"
        );
        process::exit(2);
    };
    let hint = args
        .next()
        .as_deref()
        .map(parse_hint)
        .unwrap_or(MediaSourceHint::Auto);

    let source = source_from_uri_with_hint(&uri, hint).unwrap_or_else(|error| {
        eprintln!("source open failed: {error}");
        process::exit(1);
    });
    let demuxer = Demuxer::open_source(source).unwrap_or_else(|error| {
        eprintln!("source-backed ffmpeg probe failed: {error}");
        process::exit(1);
    });

    print_probe(demuxer.probe());
}

fn parse_hint(value: &str) -> MediaSourceHint {
    match value {
        "auto" => MediaSourceHint::Auto,
        "file" => MediaSourceHint::LocalFile,
        "http" => MediaSourceHint::Http,
        _ => {
            eprintln!("hint must be one of: auto, file, http");
            process::exit(2);
        }
    }
}

fn print_probe(probe: &MediaProbe) {
    println!("Erika FFmpeg source-backed probe");
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

    for subtitle in &probe.subtitles {
        println!(
            "subtitle #{}: source={:?} removable={} language={} title={}",
            subtitle.id,
            subtitle.source,
            subtitle.can_remove(),
            subtitle.language.as_deref().unwrap_or("-"),
            subtitle.title.as_deref().unwrap_or("-"),
        );
    }
}
