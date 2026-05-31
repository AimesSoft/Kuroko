use std::env;
use std::process;

use kuroko::TrackKind;
use kuroko::ffmpeg::{DecoderConfig, DecoderOutputFrame, Demuxer, StreamSelection};

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!(
            "usage: cargo run -p ffmpeg_videotoolbox -- <media-path-or-uri> [frame-count] [stream-index]"
        );
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

    let mut demuxer = Demuxer::open_uri(&uri).unwrap_or_else(|error| {
        eprintln!("open failed: {error}");
        process::exit(1);
    });
    let stream_index = args
        .next()
        .map(|value| {
            value
                .parse::<i32>()
                .unwrap_or_else(|_| usage_error("stream-index must be an integer"))
        })
        .or_else(|| {
            demuxer
                .probe()
                .tracks
                .iter()
                .find(|track| track.kind == TrackKind::Video)
                .map(|track| track.id as i32)
        })
        .unwrap_or_else(|| usage_error("no video stream found"));

    demuxer
        .set_stream_selection(StreamSelection::only([stream_index]))
        .unwrap_or_else(|error| {
            eprintln!("stream selection failed: {error}");
            process::exit(1);
        });

    let parameters = demuxer
        .codec_parameters(stream_index)
        .unwrap_or_else(|error| {
            eprintln!("codec parameters failed: {error}");
            process::exit(1);
        });
    let mut decoder =
        kuroko::ffmpeg::Decoder::open_with_config(parameters, DecoderConfig::videotoolbox())
            .unwrap_or_else(|error| {
                eprintln!("VideoToolbox decoder open failed: {error}");
                process::exit(1);
            });

    println!("Kuroko FFmpeg VideoToolbox decode");
    println!("uri: {}", demuxer.probe().uri);
    println!("stream: {stream_index}");
    println!("backend: {:?}", decoder.backend());

    let mut decoded = 0usize;
    while decoded < frame_limit {
        match demuxer.read_packet() {
            Ok(Some(packet)) => {
                decoder.send_packet(&packet).unwrap_or_else(|error| {
                    eprintln!("send packet failed: {error}");
                    process::exit(1);
                });
                drain_frames(&mut decoder, &mut decoded, frame_limit);
            }
            Ok(None) => {
                decoder.send_eof().unwrap_or_else(|error| {
                    eprintln!("send eof failed: {error}");
                    process::exit(1);
                });
                drain_frames(&mut decoder, &mut decoded, frame_limit);
                break;
            }
            Err(error) => {
                eprintln!("read packet failed: {error}");
                process::exit(1);
            }
        }
    }
}

fn drain_frames(decoder: &mut kuroko::ffmpeg::Decoder, decoded: &mut usize, limit: usize) {
    loop {
        match decoder.receive_frame() {
            Ok(DecoderOutputFrame::Frame(frame)) => {
                let system_memory = if frame.is_videotoolbox() {
                    frame
                        .transfer_to_system_memory()
                        .ok()
                        .and_then(|frame| frame.pixel_format())
                        .unwrap_or_else(|| "transfer-failed".to_string())
                } else {
                    "-".to_string()
                };
                println!(
                    "  frame {:04} pts={} {}x{} pix_fmt={} hw={} hw_ctx={} cpu_fmt={} primaries={:?} transfer={:?}",
                    *decoded,
                    frame
                        .pts()
                        .map_or("-".to_string(), |pts| format!("{:.6}", pts.seconds())),
                    frame.width(),
                    frame.height(),
                    frame
                        .pixel_format()
                        .unwrap_or_else(|| format!("raw:{}", frame.raw_pixel_format())),
                    frame.is_videotoolbox(),
                    frame.has_hw_frames_context(),
                    system_memory,
                    frame.color_primaries(),
                    frame.transfer_function(),
                );
                *decoded += 1;
                if *decoded >= limit {
                    return;
                }
            }
            Ok(DecoderOutputFrame::NeedMoreInput) | Ok(DecoderOutputFrame::EndOfStream) => return,
            Err(error) => {
                eprintln!("receive frame failed: {error}");
                process::exit(1);
            }
        }
    }
}

fn usage_error(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2);
}
