use std::process;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use kuroko::danmaku_next2::engine::{
    EngineCommand, RenderFrameInput, create_engine, lookup_engine, poll_frame_ready,
    readback_frame_bgra, remove_engine,
};

fn main() {
    let handle = create_engine(640, 360).unwrap_or_else(|error| {
        eprintln!("next2 engine create failed: {error}");
        process::exit(1);
    });

    let result = run_smoke(handle);
    if let Some(entry) = remove_engine(handle) {
        let _ = entry.cmd_tx.send(EngineCommand::Stop);
    }

    let frame = result.unwrap_or_else(|error| {
        eprintln!("{error}");
        process::exit(1);
    });

    let nonzero_alpha = frame
        .pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[3] != 0)
        .count();
    println!(
        "Next2 danmaku smoke: {}x{}, {} bytes, nonzero alpha pixels {}",
        frame.width,
        frame.height,
        frame.pixels.len(),
        nonzero_alpha
    );

    if frame.width != 640 || frame.height != 360 || nonzero_alpha == 0 {
        eprintln!("next2 smoke rendered an empty or malformed frame");
        process::exit(1);
    }
}

fn run_smoke(handle: u64) -> Result<kuroko::danmaku_next2::engine::Next2ReadbackFrame, String> {
    let entry = lookup_engine(handle).ok_or_else(|| "next2 engine handle missing".to_string())?;
    let (reply_tx, reply_rx) = mpsc::channel();
    entry
        .cmd_tx
        .send(EngineCommand::SetFrame {
            input: RenderFrameInput {
                frame_json: r#"{"items":[{"text":"Kuroko Next2 SDF Danmaku","x":64.0,"y":96.0,"color_argb":-1,"font_size_multiplier":1.0}]}"#.to_string(),
                font_size: 36.0,
                outline_width: 1.0,
                shadow_style: 2,
                opacity: 1.0,
                custom_font_family: String::new(),
                custom_font_file_path: String::new(),
            },
            reply: reply_tx,
        })
        .map_err(|error| format!("next2 set frame send failed: {error}"))?;
    match reply_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(true) => {}
        Ok(false) => return Err("next2 renderer rejected frame json".to_string()),
        Err(error) => return Err(format!("next2 frame setup timed out: {error}")),
    }

    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) {
        if poll_frame_ready(handle) {
            if let Some(frame) = readback_frame_bgra(handle) {
                return Ok(frame);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    Err("next2 renderer did not produce a frame within 30s".to_string())
}
