use std::env;
use std::process;

use erika::TrackKind;
use erika::ffmpeg::{AudioResampler, DecoderOutputFrame, Demuxer, PcmFormat, StreamSelection};

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!(
            "usage: cargo run -p ffmpeg_audio_decode -- <media-path-or-uri> [pcm-frame-count] [stream-index]"
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
                .find(|track| track.kind == TrackKind::Audio)
                .map(|track| track.id as i32)
        })
        .unwrap_or_else(|| usage_error("no audio stream found"));

    demuxer
        .set_stream_selection(StreamSelection::only([stream_index]))
        .unwrap_or_else(|error| {
            eprintln!("stream selection failed: {error}");
            process::exit(1);
        });
    let mut decoder = demuxer.open_decoder(stream_index).unwrap_or_else(|error| {
        eprintln!("decoder open failed: {error}");
        process::exit(1);
    });

    println!("Erika FFmpeg audio decode");
    println!("uri: {}", demuxer.probe().uri);
    println!("stream: {stream_index}");
    for audio in &demuxer.probe().audio {
        println!(
            "audio #{}: codec={} rate={} channels={} sample_fmt={}",
            audio.track_id,
            audio.codec.as_deref().unwrap_or("unknown"),
            audio.sample_rate,
            audio.channels,
            audio.sample_format.as_deref().unwrap_or("unknown"),
        );
    }

    let mut resampler = None;
    let mut decoded = 0usize;
    while decoded < frame_limit {
        match demuxer.read_packet() {
            Ok(Some(packet)) => {
                decoder.send_packet(&packet).unwrap_or_else(|error| {
                    eprintln!("send packet failed: {error}");
                    process::exit(1);
                });
                drain_pcm_frames(&mut decoder, &mut resampler, &mut decoded, frame_limit);
            }
            Ok(None) => {
                decoder.send_eof().unwrap_or_else(|error| {
                    eprintln!("send eof failed: {error}");
                    process::exit(1);
                });
                drain_pcm_frames(&mut decoder, &mut resampler, &mut decoded, frame_limit);
                break;
            }
            Err(error) => {
                eprintln!("read packet failed: {error}");
                process::exit(1);
            }
        }
    }
}

fn drain_pcm_frames(
    decoder: &mut erika::ffmpeg::Decoder,
    resampler: &mut Option<AudioResampler>,
    decoded: &mut usize,
    limit: usize,
) {
    loop {
        match decoder.receive_frame() {
            Ok(DecoderOutputFrame::Frame(frame)) => {
                let output_format = PcmFormat::default();
                let resampler = resampler.get_or_insert_with(|| {
                    AudioResampler::new_from_frame(&frame, output_format).unwrap_or_else(|error| {
                        eprintln!("resampler create failed: {error}");
                        process::exit(1);
                    })
                });
                let pcm = resampler.convert(&frame).unwrap_or_else(|error| {
                    eprintln!("resample failed: {error}");
                    process::exit(1);
                });
                let peak = pcm
                    .samples
                    .iter()
                    .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
                println!(
                    "  pcm {:04} pts={:?} in={}Hz/{}ch/{} samples={} out={}Hz/{}ch frames={} peak={:.3}",
                    *decoded,
                    pcm.pts,
                    frame.sample_rate(),
                    frame.channel_count(),
                    frame
                        .sample_format()
                        .unwrap_or_else(|| format!("raw:{}", frame.raw_sample_format())),
                    frame.sample_count(),
                    pcm.format.sample_rate,
                    pcm.format.channels,
                    pcm.frames,
                    peak,
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
