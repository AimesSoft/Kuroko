use std::env;
use std::ffi::CString;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use kuroko_capi::{
    KurokoEvent, KurokoEventKind, KurokoState, KurokoStatus, kuroko_close, kuroko_create,
    kuroko_destroy, kuroko_open, kuroko_play, kuroko_poll_event, kuroko_state, kuroko_stop,
};

fn main() {
    let Some(uri) = env::args().nth(1) else {
        eprintln!("usage: cargo run -p capi_smoke -- <media-path-or-uri>");
        process::exit(2);
    };
    let uri = CString::new(uri).unwrap_or_else(|_| {
        eprintln!("media URI contains an interior NUL byte");
        process::exit(2);
    });

    let player = kuroko_create();
    if player.is_null() {
        eprintln!("kuroko_create returned null");
        process::exit(1);
    }

    let result = unsafe { run_smoke(player, &uri) };
    unsafe { kuroko_destroy(player) };
    if let Err(error) = result {
        eprintln!("C API smoke failed: {error}");
        process::exit(1);
    }
}

unsafe fn run_smoke(player: *mut kuroko_capi::KurokoHandle, uri: &CString) -> Result<(), String> {
    ensure(unsafe { kuroko_open(player, uri.as_ptr()) }, "open")?;
    wait_for_ready(player)?;
    ensure(unsafe { kuroko_play(player) }, "play")?;
    wait_for_position(player)?;
    ensure(unsafe { kuroko_stop(player) }, "stop")?;
    ensure(unsafe { kuroko_close(player) }, "close")?;

    let mut state = KurokoState::Idle;
    ensure(unsafe { kuroko_state(player, &mut state) }, "state")?;
    if state != KurokoState::Closed {
        return Err(format!("expected Closed state, got {state:?}"));
    }
    println!("C API smoke done: final_state={state:?}");
    Ok(())
}

fn wait_for_ready(player: *mut kuroko_capi::KurokoHandle) -> Result<(), String> {
    let started = Instant::now();
    let mut saw_tracks = false;
    let mut saw_video = false;
    loop {
        if started.elapsed() > Duration::from_secs(5) {
            return Err("timed out waiting for Ready event".to_string());
        }
        match poll_event(player)? {
            Some(event) => match event.kind {
                KurokoEventKind::StateChanged if event.state == KurokoState::Ready => {
                    if !saw_tracks || !saw_video {
                        return Err(format!(
                            "ready before required probe events tracks={saw_tracks} video={saw_video}"
                        ));
                    }
                    return Ok(());
                }
                KurokoEventKind::TracksChanged => {
                    saw_tracks = event.tracks.video > 0 || event.tracks.audio > 0;
                    println!(
                        "tracks: video={} audio={} subtitle={}",
                        event.tracks.video, event.tracks.audio, event.tracks.subtitle
                    );
                }
                KurokoEventKind::VideoParamsChanged => {
                    saw_video = event.video.width > 0 && event.video.height > 0;
                    println!(
                        "video: {}x{} transfer={}",
                        event.video.width, event.video.height, event.video.transfer
                    );
                }
                KurokoEventKind::Error => {
                    return Err("player emitted error while opening".to_string());
                }
                _ => {}
            },
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn wait_for_position(player: *mut kuroko_capi::KurokoHandle) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(5) {
            return Err("timed out waiting for position event".to_string());
        }
        match poll_event(player)? {
            Some(event) => match event.kind {
                KurokoEventKind::PositionChanged if event.position_micros > 0 => {
                    println!("position: {}us", event.position_micros);
                    return Ok(());
                }
                KurokoEventKind::Error => {
                    return Err("player emitted error while playing".to_string());
                }
                _ => {}
            },
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn poll_event(player: *mut kuroko_capi::KurokoHandle) -> Result<Option<KurokoEvent>, String> {
    let mut event = KurokoEvent::default();
    match unsafe { kuroko_poll_event(player, &mut event) } {
        KurokoStatus::Ok => Ok(Some(event)),
        KurokoStatus::NoEvent => Ok(None),
        status => Err(format!("poll_event returned {status:?}")),
    }
}

fn ensure(status: KurokoStatus, operation: &str) -> Result<(), String> {
    if status == KurokoStatus::Ok {
        Ok(())
    } else {
        Err(format!("{operation} returned {status:?}"))
    }
}
