use std::env;
use std::process;

use erika::TrackKind;
use erika::ffmpeg::{DecoderConfig, DecoderOutputFrame, Demuxer, StreamSelection};
use erika::renderer::metal::{MetalRenderer, VideoFrameTextureSource};

fn main() {
    let mut args = env::args().skip(1);
    let Some(uri) = args.next() else {
        eprintln!(
            "usage: cargo run -p metal_import_videotoolbox -- <media-path-or-uri> [stream-index]"
        );
        process::exit(2);
    };

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
        erika::ffmpeg::Decoder::open_with_config(parameters, DecoderConfig::videotoolbox())
            .unwrap_or_else(|error| {
                eprintln!("VideoToolbox decoder open failed: {error}");
                process::exit(1);
            });
    let mut renderer = MetalRenderer::new().unwrap_or_else(|error| {
        eprintln!("Metal renderer create failed: {error}");
        process::exit(1);
    });

    println!("Erika VideoToolbox -> CVMetalTextureCache import");
    println!("uri: {}", demuxer.probe().uri);
    println!("stream: {stream_index}");

    loop {
        match demuxer.read_packet() {
            Ok(Some(packet)) => {
                decoder.send_packet(&packet).unwrap_or_else(|error| {
                    eprintln!("send packet failed: {error}");
                    process::exit(1);
                });
                if import_first_frame(&mut decoder, &mut renderer) {
                    return;
                }
            }
            Ok(None) => {
                decoder.send_eof().unwrap_or_else(|error| {
                    eprintln!("send eof failed: {error}");
                    process::exit(1);
                });
                if import_first_frame(&mut decoder, &mut renderer) {
                    return;
                }
                eprintln!("no video frame decoded before EOF");
                process::exit(1);
            }
            Err(error) => {
                eprintln!("read packet failed: {error}");
                process::exit(1);
            }
        }
    }
}

fn import_first_frame(decoder: &mut erika::ffmpeg::Decoder, renderer: &mut MetalRenderer) -> bool {
    loop {
        match decoder.receive_frame() {
            Ok(DecoderOutputFrame::Frame(frame)) => {
                let Some(pixel_buffer) = frame.videotoolbox_pixel_buffer() else {
                    eprintln!("decoded frame was not backed by a VideoToolbox CVPixelBuffer");
                    process::exit(1);
                };
                let imported = unsafe {
                    renderer.import_video_frame_textures(VideoFrameTextureSource::new(
                        pixel_buffer.raw(),
                        pixel_buffer.width(),
                        pixel_buffer.height(),
                    ))
                }
                .unwrap_or_else(|error| {
                    eprintln!("Metal texture import failed: {error}");
                    process::exit(1);
                });

                let info = imported.info();
                println!(
                    "frame pts={} {}x{} cv={} format={:?} planes={} peak_nits={}",
                    frame
                        .pts()
                        .map_or("-".to_string(), |pts| format!("{:.6}", pts.seconds())),
                    info.width,
                    info.height,
                    info.pixel_format_fourcc,
                    info.format,
                    imported.plane_count(),
                    frame
                        .hdr_metadata()
                        .and_then(|metadata| metadata.nominal_peak_nits())
                        .map_or("-".to_string(), |peak| format!("{peak:.0}")),
                );
                if let Some(metadata) = frame.hdr_metadata() {
                    if let Some(mastering) = metadata.mastering_display {
                        println!(
                            "  mastering: min={} max={}",
                            mastering
                                .min_luminance_nits
                                .map_or("-".to_string(), |value| format!("{value:.4}")),
                            mastering
                                .max_luminance_nits
                                .map_or("-".to_string(), |value| format!("{value:.0}")),
                        );
                    }
                    if let Some(content_light) = metadata.content_light {
                        println!(
                            "  content light: MaxCLL={} MaxFALL={}",
                            content_light.max_content_light_level_nits,
                            content_light.max_frame_average_light_level_nits,
                        );
                    }
                }
                for plane in &info.planes {
                    println!(
                        "  plane {}: {}x{} metal={}",
                        plane.index, plane.width, plane.height, plane.metal_pixel_format,
                    );
                }
                return true;
            }
            Ok(DecoderOutputFrame::NeedMoreInput) => return false,
            Ok(DecoderOutputFrame::EndOfStream) => return false,
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
