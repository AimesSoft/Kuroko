use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use crossbeam_channel::Receiver;
#[cfg(target_os = "macos")]
use kuroko::presenter::{PresenterConfig, PresenterRuntime, PresenterStats};
use kuroko::{
    FlutterTextureHandle, FlutterTextureKind, MediaRequest, MetalSurfaceHandle, PlatformSurface,
    Player, PlayerConfig, PlayerEvent, PlayerState, TrackKind, TransferFunction, WgpuSurfaceHandle,
    WgpuSurfaceKind,
};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KurokoStatus {
    Ok = 0,
    NullPointer = 1,
    InvalidUtf8 = 2,
    PlayerError = 3,
    Panic = 4,
    NoEvent = 5,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KurokoState {
    Idle = 0,
    Opening = 1,
    Ready = 2,
    Playing = 3,
    Paused = 4,
    Stopped = 5,
    Closed = 6,
    Error = 7,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KurokoEventKind {
    None = 0,
    StateChanged = 1,
    DurationChanged = 2,
    PositionChanged = 3,
    TracksChanged = 4,
    BufferingChanged = 5,
    VideoParamsChanged = 6,
    SurfaceAttached = 7,
    SurfaceDetached = 8,
    Error = 9,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KurokoWgpuSurfaceKind {
    Unknown = 0,
    MacOsNsView = 1,
    MacOsCaMetalLayer = 2,
    IosUiView = 3,
    WindowsHwnd = 4,
    XlibWindow = 5,
    WaylandSurface = 6,
    AndroidNativeWindow = 7,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KurokoFlutterTextureKind {
    Unknown = 0,
    MacOsTextureRegistrar = 1,
    IosTextureRegistrar = 2,
    AndroidSurfaceTexture = 3,
    WindowsTextureRegistrar = 4,
    LinuxTextureRegistrar = 5,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KurokoVideoParams {
    pub width: u32,
    pub height: u32,
    pub primaries: u32,
    pub transfer: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KurokoTrackCounts {
    pub video: u32,
    pub audio: u32,
    pub subtitle: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KurokoEvent {
    pub kind: KurokoEventKind,
    pub status: KurokoStatus,
    pub state: KurokoState,
    pub duration_micros: i64,
    pub position_micros: u64,
    pub buffering: bool,
    pub video: KurokoVideoParams,
    pub tracks: KurokoTrackCounts,
}

impl Default for KurokoEvent {
    fn default() -> Self {
        Self {
            kind: KurokoEventKind::None,
            status: KurokoStatus::Ok,
            state: KurokoState::Idle,
            duration_micros: -1,
            position_micros: 0,
            buffering: false,
            video: KurokoVideoParams::default(),
            tracks: KurokoTrackCounts::default(),
        }
    }
}

pub struct KurokoHandle {
    player: Player,
    events: Receiver<PlayerEvent>,
}

#[cfg(target_os = "macos")]
pub struct KurokoPresenterHandle {
    presenter: PresenterRuntime,
    events: Receiver<PlayerEvent>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KurokoPresenterStats {
    pub decoded_video_frames: u64,
    pub rendered_video_frames: u64,
    pub rendered_test_frames: u64,
    pub pushed_audio_frames: u64,
    pub overlay_frames: u64,
    pub import_failures: u64,
    pub render_failures: u64,
    pub audio_failures: u64,
}

#[unsafe(no_mangle)]
pub extern "C" fn kuroko_create() -> *mut KurokoHandle {
    let player = Player::new(PlayerConfig::default());
    let events = player.subscribe();
    Box::into_raw(Box::new(KurokoHandle { player, events }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_destroy(handle: *mut KurokoHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_open(
    handle: *mut KurokoHandle,
    uri: *const c_char,
) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        let uri = match c_string(uri) {
            Ok(uri) => uri,
            Err(status) => return status,
        };
        status_from_player_result(handle.player.open(MediaRequest::new(uri)))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_play(handle: *mut KurokoHandle) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.play())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_pause(handle: *mut KurokoHandle) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.pause())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_stop(handle: *mut KurokoHandle) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.stop())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_close(handle: *mut KurokoHandle) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.close())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_seek(
    handle: *mut KurokoHandle,
    position_micros: u64,
) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.seek(Duration::from_micros(position_micros)))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_attach_metal_layer(
    handle: *mut KurokoHandle,
    raw_layer: u64,
    width: u32,
    height: u32,
    scale: f64,
) -> KurokoStatus {
    if raw_layer == 0 {
        return KurokoStatus::NullPointer;
    }
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.attach_surface(PlatformSurface::Metal(
            MetalSurfaceHandle::new(raw_layer, width, height, scale),
        )))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_attach_wgpu_surface(
    handle: *mut KurokoHandle,
    kind: KurokoWgpuSurfaceKind,
    raw_window: u64,
    raw_display: u64,
    width: u32,
    height: u32,
    scale: f64,
) -> KurokoStatus {
    if raw_window == 0 {
        return KurokoStatus::NullPointer;
    }
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.attach_surface(PlatformSurface::Wgpu(
            WgpuSurfaceHandle::new(
                wgpu_surface_kind_from_c(kind),
                raw_window,
                raw_display,
                width,
                height,
                scale,
            ),
        )))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_attach_flutter_texture(
    handle: *mut KurokoHandle,
    kind: KurokoFlutterTextureKind,
    texture_id: i64,
    width: u32,
    height: u32,
    scale: f64,
) -> KurokoStatus {
    if texture_id < 0 {
        return KurokoStatus::NullPointer;
    }
    with_handle_mut(handle, |handle| {
        status_from_player_result(
            handle
                .player
                .attach_surface(PlatformSurface::FlutterTexture(FlutterTextureHandle::new(
                    flutter_texture_kind_from_c(kind),
                    texture_id,
                    width,
                    height,
                    scale,
                ))),
        )
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_detach_surface(handle: *mut KurokoHandle) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.detach_surface())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_state(
    handle: *mut KurokoHandle,
    out_state: *mut KurokoState,
) -> KurokoStatus {
    if out_state.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_handle_mut(handle, |handle| {
        unsafe { *out_state = state_to_c(handle.player.state()) };
        KurokoStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_poll_event(
    handle: *mut KurokoHandle,
    out_event: *mut KurokoEvent,
) -> KurokoStatus {
    if out_event.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_handle_mut(handle, |handle| match handle.events.try_recv() {
        Ok(event) => {
            unsafe { *out_event = event_to_c(event) };
            KurokoStatus::Ok
        }
        Err(crossbeam_channel::TryRecvError::Empty) => KurokoStatus::NoEvent,
        Err(crossbeam_channel::TryRecvError::Disconnected) => KurokoStatus::PlayerError,
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub extern "C" fn kuroko_presenter_create() -> *mut KurokoPresenterHandle {
    match PresenterRuntime::new(PresenterConfig::default()) {
        Ok(presenter) => {
            let events = presenter.player().subscribe();
            Box::into_raw(Box::new(KurokoPresenterHandle { presenter, events }))
        }
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(not(target_os = "macos"))]
#[unsafe(no_mangle)]
pub extern "C" fn kuroko_presenter_create() -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_destroy(handle: *mut KurokoPresenterHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

#[cfg(not(target_os = "macos"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_destroy(_handle: *mut std::ffi::c_void) {}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_open(
    handle: *mut KurokoPresenterHandle,
    uri: *const c_char,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        let uri = match c_string(uri) {
            Ok(uri) => uri,
            Err(status) => return status,
        };
        status_from_player_result(handle.presenter.open(MediaRequest::new(uri)))
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_play(handle: *mut KurokoPresenterHandle) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.play())
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_pause(
    handle: *mut KurokoPresenterHandle,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.pause())
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_stop(handle: *mut KurokoPresenterHandle) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.stop())
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_close(
    handle: *mut KurokoPresenterHandle,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.close())
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_seek(
    handle: *mut KurokoPresenterHandle,
    position_micros: u64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(
            handle
                .presenter
                .seek(Duration::from_micros(position_micros)),
        )
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_attach_metal_layer(
    handle: *mut KurokoPresenterHandle,
    raw_layer: u64,
    width: u32,
    height: u32,
    scale: f64,
) -> KurokoStatus {
    if raw_layer == 0 {
        return KurokoStatus::NullPointer;
    }
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.attach_surface(PlatformSurface::Metal(
            MetalSurfaceHandle::new(raw_layer, width, height, scale),
        )))
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_resize_surface(
    handle: *mut KurokoPresenterHandle,
    width: u32,
    height: u32,
    scale: f64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.resize_surface(width, height, scale))
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_detach_surface(
    handle: *mut KurokoPresenterHandle,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.detach_surface())
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_render_tick(
    handle: *mut KurokoPresenterHandle,
    time_seconds: f64,
    out_stats: *mut KurokoPresenterStats,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        match handle.presenter.render_tick(time_seconds) {
            Ok(stats) => {
                if !out_stats.is_null() {
                    unsafe { *out_stats = presenter_stats_to_c(stats) };
                }
                KurokoStatus::Ok
            }
            Err(_) => KurokoStatus::PlayerError,
        }
    })
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_poll_event(
    handle: *mut KurokoPresenterHandle,
    out_event: *mut KurokoEvent,
) -> KurokoStatus {
    if out_event.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_presenter_mut(handle, |handle| match handle.events.try_recv() {
        Ok(event) => {
            unsafe { *out_event = event_to_c(event) };
            KurokoStatus::Ok
        }
        Err(crossbeam_channel::TryRecvError::Empty) => KurokoStatus::NoEvent,
        Err(crossbeam_channel::TryRecvError::Disconnected) => KurokoStatus::PlayerError,
    })
}

fn with_handle_mut(
    handle: *mut KurokoHandle,
    f: impl FnOnce(&mut KurokoHandle) -> KurokoStatus,
) -> KurokoStatus {
    if handle.is_null() {
        return KurokoStatus::NullPointer;
    }
    match catch_unwind(AssertUnwindSafe(|| f(unsafe { &mut *handle }))) {
        Ok(status) => status,
        Err(_) => KurokoStatus::Panic,
    }
}

#[cfg(target_os = "macos")]
fn with_presenter_mut(
    handle: *mut KurokoPresenterHandle,
    f: impl FnOnce(&mut KurokoPresenterHandle) -> KurokoStatus,
) -> KurokoStatus {
    if handle.is_null() {
        return KurokoStatus::NullPointer;
    }
    match catch_unwind(AssertUnwindSafe(|| f(unsafe { &mut *handle }))) {
        Ok(status) => status,
        Err(_) => KurokoStatus::Panic,
    }
}

fn c_string(ptr: *const c_char) -> Result<String, KurokoStatus> {
    if ptr.is_null() {
        return Err(KurokoStatus::NullPointer);
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(str::to_string)
        .map_err(|_| KurokoStatus::InvalidUtf8)
}

fn status_from_player_result(result: kuroko::Result<()>) -> KurokoStatus {
    match result {
        Ok(()) => KurokoStatus::Ok,
        Err(_) => KurokoStatus::PlayerError,
    }
}

fn event_to_c(event: PlayerEvent) -> KurokoEvent {
    match event {
        PlayerEvent::StateChanged(state) => KurokoEvent {
            kind: KurokoEventKind::StateChanged,
            state: state_to_c(state),
            ..KurokoEvent::default()
        },
        PlayerEvent::DurationChanged(duration) => KurokoEvent {
            kind: KurokoEventKind::DurationChanged,
            duration_micros: duration.map(duration_micros_i64).unwrap_or(-1),
            ..KurokoEvent::default()
        },
        PlayerEvent::PositionChanged(position) => KurokoEvent {
            kind: KurokoEventKind::PositionChanged,
            position_micros: duration_micros_u64(position),
            ..KurokoEvent::default()
        },
        PlayerEvent::TracksChanged(tracks) => {
            let mut counts = KurokoTrackCounts::default();
            for track in tracks {
                match track.kind {
                    TrackKind::Video => counts.video = counts.video.saturating_add(1),
                    TrackKind::Audio => counts.audio = counts.audio.saturating_add(1),
                    TrackKind::Subtitle => counts.subtitle = counts.subtitle.saturating_add(1),
                }
            }
            KurokoEvent {
                kind: KurokoEventKind::TracksChanged,
                tracks: counts,
                ..KurokoEvent::default()
            }
        }
        PlayerEvent::BufferingChanged(buffering) => KurokoEvent {
            kind: KurokoEventKind::BufferingChanged,
            buffering,
            ..KurokoEvent::default()
        },
        PlayerEvent::VideoParamsChanged(params) => KurokoEvent {
            kind: KurokoEventKind::VideoParamsChanged,
            video: KurokoVideoParams {
                width: params.width,
                height: params.height,
                primaries: params.primaries as u32,
                transfer: transfer_to_c(params.transfer),
            },
            ..KurokoEvent::default()
        },
        PlayerEvent::SurfaceAttached(_) => KurokoEvent {
            kind: KurokoEventKind::SurfaceAttached,
            ..KurokoEvent::default()
        },
        PlayerEvent::SurfaceDetached => KurokoEvent {
            kind: KurokoEventKind::SurfaceDetached,
            ..KurokoEvent::default()
        },
        PlayerEvent::Error(_) => KurokoEvent {
            kind: KurokoEventKind::Error,
            status: KurokoStatus::PlayerError,
            ..KurokoEvent::default()
        },
    }
}

fn state_to_c(state: PlayerState) -> KurokoState {
    match state {
        PlayerState::Idle => KurokoState::Idle,
        PlayerState::Opening => KurokoState::Opening,
        PlayerState::Ready => KurokoState::Ready,
        PlayerState::Playing => KurokoState::Playing,
        PlayerState::Paused => KurokoState::Paused,
        PlayerState::Stopped => KurokoState::Stopped,
        PlayerState::Closed => KurokoState::Closed,
        PlayerState::Error => KurokoState::Error,
    }
}

fn transfer_to_c(transfer: TransferFunction) -> u32 {
    match transfer {
        TransferFunction::Unknown => 0,
        TransferFunction::Srgb => 1,
        TransferFunction::Bt1886 => 2,
        TransferFunction::Pq => 3,
        TransferFunction::Hlg => 4,
    }
}

fn wgpu_surface_kind_from_c(kind: KurokoWgpuSurfaceKind) -> WgpuSurfaceKind {
    match kind {
        KurokoWgpuSurfaceKind::Unknown => WgpuSurfaceKind::Unknown,
        KurokoWgpuSurfaceKind::MacOsNsView => WgpuSurfaceKind::MacOsNsView,
        KurokoWgpuSurfaceKind::MacOsCaMetalLayer => WgpuSurfaceKind::MacOsCaMetalLayer,
        KurokoWgpuSurfaceKind::IosUiView => WgpuSurfaceKind::IosUiView,
        KurokoWgpuSurfaceKind::WindowsHwnd => WgpuSurfaceKind::WindowsHwnd,
        KurokoWgpuSurfaceKind::XlibWindow => WgpuSurfaceKind::XlibWindow,
        KurokoWgpuSurfaceKind::WaylandSurface => WgpuSurfaceKind::WaylandSurface,
        KurokoWgpuSurfaceKind::AndroidNativeWindow => WgpuSurfaceKind::AndroidNativeWindow,
    }
}

fn flutter_texture_kind_from_c(kind: KurokoFlutterTextureKind) -> FlutterTextureKind {
    match kind {
        KurokoFlutterTextureKind::Unknown => FlutterTextureKind::Unknown,
        KurokoFlutterTextureKind::MacOsTextureRegistrar => {
            FlutterTextureKind::MacOsTextureRegistrar
        }
        KurokoFlutterTextureKind::IosTextureRegistrar => FlutterTextureKind::IosTextureRegistrar,
        KurokoFlutterTextureKind::AndroidSurfaceTexture => {
            FlutterTextureKind::AndroidSurfaceTexture
        }
        KurokoFlutterTextureKind::WindowsTextureRegistrar => {
            FlutterTextureKind::WindowsTextureRegistrar
        }
        KurokoFlutterTextureKind::LinuxTextureRegistrar => {
            FlutterTextureKind::LinuxTextureRegistrar
        }
    }
}

fn duration_micros_i64(duration: Duration) -> i64 {
    duration.as_micros().min(i64::MAX as u128) as i64
}

fn duration_micros_u64(duration: Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}

#[cfg(target_os = "macos")]
fn presenter_stats_to_c(stats: PresenterStats) -> KurokoPresenterStats {
    KurokoPresenterStats {
        decoded_video_frames: stats.decoded_video_frames,
        rendered_video_frames: stats.rendered_video_frames,
        rendered_test_frames: stats.rendered_test_frames,
        pushed_audio_frames: stats.pushed_audio_frames,
        overlay_frames: stats.overlay_frames,
        import_failures: stats.import_failures,
        render_failures: stats.render_failures,
        audio_failures: stats.audio_failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_event_counts_tracks() {
        let event = event_to_c(PlayerEvent::TracksChanged(vec![
            kuroko::TrackInfo {
                id: 0,
                kind: TrackKind::Video,
                title: None,
                language: None,
                codec: None,
            },
            kuroko::TrackInfo {
                id: 1,
                kind: TrackKind::Audio,
                title: None,
                language: None,
                codec: None,
            },
        ]));

        assert_eq!(event.kind, KurokoEventKind::TracksChanged);
        assert_eq!(event.tracks.video, 1);
        assert_eq!(event.tracks.audio, 1);
        assert_eq!(event.tracks.subtitle, 0);
    }

    #[test]
    fn null_handle_is_rejected() {
        assert_eq!(
            unsafe { kuroko_play(std::ptr::null_mut()) },
            KurokoStatus::NullPointer
        );
    }

    #[test]
    fn c_surface_attach_emits_events() {
        let handle = kuroko_create();
        assert!(!handle.is_null());

        let status = unsafe { kuroko_attach_metal_layer(handle, 42, 1920, 1080, 2.0) };
        assert_eq!(status, KurokoStatus::Ok);

        let mut event = KurokoEvent::default();
        let status = unsafe { kuroko_poll_event(handle, &mut event) };
        assert_eq!(status, KurokoStatus::Ok);
        assert_eq!(event.kind, KurokoEventKind::SurfaceAttached);

        let status = unsafe { kuroko_detach_surface(handle) };
        assert_eq!(status, KurokoStatus::Ok);
        let status = unsafe { kuroko_poll_event(handle, &mut event) };
        assert_eq!(status, KurokoStatus::Ok);
        assert_eq!(event.kind, KurokoEventKind::SurfaceDetached);

        unsafe { kuroko_destroy(handle) };
    }

    #[test]
    fn c_wgpu_surface_attach_emits_event() {
        let handle = kuroko_create();
        assert!(!handle.is_null());

        let status = unsafe {
            kuroko_attach_wgpu_surface(
                handle,
                KurokoWgpuSurfaceKind::MacOsCaMetalLayer,
                42,
                0,
                1920,
                1080,
                2.0,
            )
        };
        assert_eq!(status, KurokoStatus::Ok);

        let mut event = KurokoEvent::default();
        let status = unsafe { kuroko_poll_event(handle, &mut event) };
        assert_eq!(status, KurokoStatus::Ok);
        assert_eq!(event.kind, KurokoEventKind::SurfaceAttached);

        unsafe { kuroko_destroy(handle) };
    }

    #[test]
    fn c_flutter_texture_attach_emits_event() {
        let handle = kuroko_create();
        assert!(!handle.is_null());

        let status = unsafe {
            kuroko_attach_flutter_texture(
                handle,
                KurokoFlutterTextureKind::MacOsTextureRegistrar,
                7,
                1280,
                720,
                2.0,
            )
        };
        assert_eq!(status, KurokoStatus::Ok);

        let mut event = KurokoEvent::default();
        let status = unsafe { kuroko_poll_event(handle, &mut event) };
        assert_eq!(status, KurokoStatus::Ok);
        assert_eq!(event.kind, KurokoEventKind::SurfaceAttached);

        unsafe { kuroko_destroy(handle) };
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn c_presenter_lifecycle_rejects_null_and_can_be_destroyed() {
        assert_eq!(
            unsafe { kuroko_presenter_play(std::ptr::null_mut()) },
            KurokoStatus::NullPointer
        );
        let handle = kuroko_presenter_create();
        assert!(!handle.is_null());
        unsafe { kuroko_presenter_destroy(handle) };
    }
}
