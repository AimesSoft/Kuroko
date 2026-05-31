use std::env;
use std::process;
use std::time::Duration;

use kuroko::ffmpeg::{Demuxer, StreamSelection};

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!(
            "usage: cargo run -p ffmpeg_demux -- <media-path-or-uri> [packet-count] [stream-index] [seek-seconds]"
        );
        process::exit(2);
    };
    let packet_limit = args
        .next()
        .map(|value| {
            value
                .parse::<usize>()
                .unwrap_or_else(|_| usage_error("packet-count must be a positive integer"))
        })
        .unwrap_or(24);
    let stream_index = args.next().map(|value| {
        value
            .parse::<i32>()
            .unwrap_or_else(|_| usage_error("stream-index must be an integer"))
    });
    let seek_seconds = args.next().map(|value| {
        value
            .parse::<f64>()
            .unwrap_or_else(|_| usage_error("seek-seconds must be a number"))
    });

    let mut demuxer = match Demuxer::open_uri(&uri) {
        Ok(demuxer) => demuxer,
        Err(error) => {
            eprintln!("demux open failed: {error}");
            process::exit(1);
        }
    };

    if let Some(stream_index) = stream_index {
        if let Err(error) = demuxer.set_stream_selection(StreamSelection::only([stream_index])) {
            eprintln!("stream selection failed: {error}");
            process::exit(1);
        }
    }
    if let Some(seek_seconds) = seek_seconds {
        if seek_seconds.is_sign_negative() || !seek_seconds.is_finite() {
            usage_error("seek-seconds must be a non-negative finite number");
        }
        if let Err(error) = demuxer.seek(Duration::from_secs_f64(seek_seconds)) {
            eprintln!("seek failed: {error}");
            process::exit(1);
        }
        println!("seek: {seek_seconds:.3}s");
    }

    let probe = demuxer.probe();
    println!("Kuroko FFmpeg demux");
    println!("uri: {}", probe.uri);
    match probe.duration {
        Some(duration) => println!("duration: {:.3}s", duration.as_secs_f64()),
        None => println!("duration: unknown"),
    }
    for track in &probe.tracks {
        println!(
            "track #{} {:?} codec={}",
            track.id,
            track.kind,
            track.codec.as_deref().unwrap_or("unknown")
        );
    }

    println!("packets:");
    for ordinal in 0..packet_limit {
        let Some(packet) = read_next_packet(&mut demuxer) else {
            println!("  eof after {ordinal} packets");
            break;
        };
        println!(
            "  {:04} stream={} pts={} dts={} dur={} key={} size={} pos={}",
            ordinal,
            packet.stream_index(),
            packet
                .pts()
                .map_or("-".to_string(), |pts| format!("{:.6}", pts.seconds())),
            packet
                .dts()
                .map_or("-".to_string(), |dts| format!("{:.6}", dts.seconds())),
            packet
                .duration_seconds()
                .map_or("-".to_string(), |duration| format!("{duration:.6}")),
            packet.is_key(),
            packet.size(),
            packet.pos().map_or("-".to_string(), |pos| pos.to_string())
        );
    }
}

fn read_next_packet(demuxer: &mut Demuxer) -> Option<kuroko::ffmpeg::Packet> {
    match demuxer.read_packet() {
        Ok(packet) => packet,
        Err(error) => {
            eprintln!("read packet failed: {error}");
            process::exit(1);
        }
    }
}

fn usage_error(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2);
}
