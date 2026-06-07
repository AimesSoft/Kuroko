use std::env;
use std::ffi::CString;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use erika_capi::{
    ErikaEvent, ErikaEventKind, ErikaState, ErikaStatus, erika_close, erika_create, erika_destroy,
    erika_open, erika_play, erika_poll_event, erika_state, erika_stop,
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

    let player = erika_create();
    if player.is_null() {
        eprintln!("erika_create returned null");
        process::exit(1);
    }

    let result = unsafe { run_smoke(player, &uri) };
    unsafe { erika_destroy(player) };
    if let Err(error) = result {
        eprintln!("C API smoke failed: {error}");
        process::exit(1);
    }
}

unsafe fn run_smoke(player: *mut erika_capi::ErikaHandle, uri: &CString) -> Result<(), String> {
    ensure(unsafe { erika_open(player, uri.as_ptr()) }, "open")?;
    wait_for_ready(player)?;
    ensure(unsafe { erika_play(player) }, "play")?;
    wait_for_position(player)?;
    ensure(unsafe { erika_stop(player) }, "stop")?;
    ensure(unsafe { erika_close(player) }, "close")?;

    let mut state = ErikaState::Idle;
    ensure(unsafe { erika_state(player, &mut state) }, "state")?;
    if state != ErikaState::Closed {
        return Err(format!("expected Closed state, got {state:?}"));
    }
    println!("C API smoke done: final_state={state:?}");
    Ok(())
}

fn wait_for_ready(player: *mut erika_capi::ErikaHandle) -> Result<(), String> {
    let started = Instant::now();
    let mut saw_tracks = false;
    let mut saw_video = false;
    loop {
        if started.elapsed() > Duration::from_secs(5) {
            return Err("timed out waiting for Ready event".to_string());
        }
        match poll_event(player)? {
            Some(event) => match event.kind {
                ErikaEventKind::StateChanged if event.state == ErikaState::Ready => {
                    if !saw_tracks || !saw_video {
                        return Err(format!(
                            "ready before required probe events tracks={saw_tracks} video={saw_video}"
                        ));
                    }
                    return Ok(());
                }
                ErikaEventKind::TracksChanged => {
                    saw_tracks = event.tracks.video > 0 || event.tracks.audio > 0;
                    println!(
                        "tracks: video={} audio={} subtitle={}",
                        event.tracks.video, event.tracks.audio, event.tracks.subtitle
                    );
                }
                ErikaEventKind::VideoParamsChanged => {
                    saw_video = event.video.width > 0 && event.video.height > 0;
                    println!(
                        "video: {}x{} transfer={}",
                        event.video.width, event.video.height, event.video.transfer
                    );
                }
                ErikaEventKind::Error => {
                    return Err("player emitted error while opening".to_string());
                }
                _ => {}
            },
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn wait_for_position(player: *mut erika_capi::ErikaHandle) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(5) {
            return Err("timed out waiting for position event".to_string());
        }
        match poll_event(player)? {
            Some(event) => match event.kind {
                ErikaEventKind::PositionChanged if event.position_micros > 0 => {
                    println!("position: {}us", event.position_micros);
                    return Ok(());
                }
                ErikaEventKind::Error => {
                    return Err("player emitted error while playing".to_string());
                }
                _ => {}
            },
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn poll_event(player: *mut erika_capi::ErikaHandle) -> Result<Option<ErikaEvent>, String> {
    let mut event = ErikaEvent::default();
    match unsafe { erika_poll_event(player, &mut event) } {
        ErikaStatus::Ok => Ok(Some(event)),
        ErikaStatus::NoEvent => Ok(None),
        status => Err(format!("poll_event returned {status:?}")),
    }
}

fn ensure(status: ErikaStatus, operation: &str) -> Result<(), String> {
    if status == ErikaStatus::Ok {
        Ok(())
    } else {
        Err(format!("{operation} returned {status:?}"))
    }
}
