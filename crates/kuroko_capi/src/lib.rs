use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use crossbeam_channel::Receiver;
use kuroko::danmaku::{
    DanmakuLayoutConfig, DanmakuShadowStyle, DanmakuTimeline, DanmakuTrackInfo, DanmakuTrackSource,
};
#[cfg(any(target_os = "macos", target_os = "ios"))]
use kuroko::presenter::{PresenterConfig, PresenterRuntime, PresenterStats};
#[cfg(any(target_os = "macos", target_os = "ios"))]
use kuroko::renderer::metal::{MetalOutputMode, MetalRendererConfig};
use kuroko::{
    FlutterTextureHandle, FlutterTextureKind, MediaRequest, MetalSurfaceHandle, PlatformSurface,
    Player, PlayerConfig, PlayerEvent, PlayerState, TrackInfo, TrackKind, TrackSelection,
    TrackSource, TransferFunction, WgpuSurfaceHandle, WgpuSurfaceKind,
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
    TrackSelectionChanged = 10,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KurokoTrackKind {
    Video = 0,
    Audio = 1,
    Subtitle = 2,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KurokoTrackSource {
    Embedded = 0,
    External = 1,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KurokoTrackSelection {
    pub video: i64,
    pub audio: i64,
    pub subtitle: i64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KurokoTrackInfo {
    pub id: i64,
    pub kind: KurokoTrackKind,
    pub source: KurokoTrackSource,
    pub selected: bool,
    pub can_remove: bool,
    pub title: *mut c_char,
    pub language: *mut c_char,
    pub codec: *mut c_char,
}

impl Default for KurokoTrackInfo {
    fn default() -> Self {
        Self {
            id: -1,
            kind: KurokoTrackKind::Video,
            source: KurokoTrackSource::Embedded,
            selected: false,
            can_remove: false,
            title: std::ptr::null_mut(),
            language: std::ptr::null_mut(),
            codec: std::ptr::null_mut(),
        }
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KurokoPresenterOutputMode {
    Sdr = 0,
    AppleEdr = 1,
}

impl KurokoPresenterOutputMode {
    fn from_raw(value: i32) -> Self {
        match value {
            1 => Self::AppleEdr,
            _ => Self::Sdr,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KurokoPresenterConfig {
    pub output_mode: i32,
    pub edr_headroom: f32,
}

impl Default for KurokoPresenterConfig {
    fn default() -> Self {
        Self {
            output_mode: KurokoPresenterOutputMode::Sdr as i32,
            edr_headroom: 1.0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KurokoDanmakuConfig {
    pub enabled: bool,
    /// NipaPlay/Flutter logical danmaku font size. Kuroko uses the NipaPlay
    /// default danmaku font and multiplies by the surface scale for glyph pixels.
    pub font_size: f32,
    pub opacity: f32,
    pub display_area: f32,
    pub scroll_duration_seconds: f32,
    pub scroll_speed_factor: f32,
    pub track_gap_ratio: f32,
    pub outline_width: f32,
    pub shadow_offset_x: f32,
    pub shadow_offset_y: f32,
    pub merge_duplicates: bool,
    pub allow_stacking: bool,
    pub allow_scroll_overwrite: bool,
    pub max_quantity: u32,
    pub max_lines_per_mode: u32,
    pub block_top: bool,
    pub block_bottom: bool,
    pub block_scroll: bool,
    pub shadow_style: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KurokoDanmakuTrackInfo {
    pub id: u64,
    pub enabled: bool,
    pub offset_micros: i64,
    pub item_count: usize,
    pub name: *mut c_char,
    pub source: *mut c_char,
}

impl Default for KurokoDanmakuTrackInfo {
    fn default() -> Self {
        Self {
            id: 0,
            enabled: false,
            offset_micros: 0,
            item_count: 0,
            name: std::ptr::null_mut(),
            source: std::ptr::null_mut(),
        }
    }
}

impl Default for KurokoDanmakuConfig {
    fn default() -> Self {
        let config = DanmakuLayoutConfig::default();
        Self {
            enabled: config.enabled,
            font_size: config.font_size,
            opacity: config.opacity,
            display_area: config.display_area,
            scroll_duration_seconds: config.scroll_duration_seconds,
            scroll_speed_factor: config.scroll_speed_factor,
            track_gap_ratio: config.track_gap_ratio,
            outline_width: config.outline_width,
            shadow_offset_x: config.shadow_offset[0],
            shadow_offset_y: config.shadow_offset[1],
            merge_duplicates: config.merge_duplicates,
            allow_stacking: config.allow_stacking,
            allow_scroll_overwrite: config.allow_scroll_overwrite,
            max_quantity: 0,
            max_lines_per_mode: 0,
            block_top: config.block_top,
            block_bottom: config.block_bottom,
            block_scroll: config.block_scroll,
            shadow_style: config.shadow_style.code(),
        }
    }
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
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
    pub danmaku_frames: u64,
    pub danmaku_items: u64,
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
pub unsafe extern "C" fn kuroko_add_external_subtitle(
    handle: *mut KurokoHandle,
    uri: *const c_char,
    out_track_id: *mut i64,
) -> KurokoStatus {
    if out_track_id.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_handle_mut(handle, |handle| {
        let uri = match c_string(uri) {
            Ok(uri) => uri,
            Err(status) => return status,
        };
        match handle.player.add_external_subtitle(uri) {
            Ok(track) => {
                unsafe { *out_track_id = track.id };
                KurokoStatus::Ok
            }
            Err(_) => KurokoStatus::PlayerError,
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_remove_subtitle_track(
    handle: *mut KurokoHandle,
    track_id: i64,
) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.remove_subtitle_track(track_id))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_select_audio_track(
    handle: *mut KurokoHandle,
    track_id: i64,
) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        status_from_player_result(handle.player.select_audio_track(track_id_option(track_id)))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_select_subtitle_track(
    handle: *mut KurokoHandle,
    track_id: i64,
) -> KurokoStatus {
    with_handle_mut(handle, |handle| {
        status_from_player_result(
            handle
                .player
                .select_subtitle_track(track_id_option(track_id)),
        )
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_track_selection(
    handle: *mut KurokoHandle,
    out_selection: *mut KurokoTrackSelection,
) -> KurokoStatus {
    if out_selection.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_handle_mut(handle, |handle| {
        unsafe { *out_selection = track_selection_to_c(handle.player.track_selection()) };
        KurokoStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_tracks(
    handle: *mut KurokoHandle,
    out_tracks: *mut KurokoTrackInfo,
    capacity: usize,
    out_len: *mut usize,
) -> KurokoStatus {
    if out_len.is_null() || (capacity > 0 && out_tracks.is_null()) {
        return KurokoStatus::NullPointer;
    }
    with_handle_mut(handle, |handle| {
        write_tracks_to_c(&handle.player.tracks(), out_tracks, capacity, out_len)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_track_info_free(track: *mut KurokoTrackInfo) {
    if track.is_null() {
        return;
    }
    let track = unsafe { &mut *track };
    free_c_string(&mut track.title);
    free_c_string(&mut track.language);
    free_c_string(&mut track.codec);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_danmaku_track_info_free(track: *mut KurokoDanmakuTrackInfo) {
    if track.is_null() {
        return;
    }
    let track = unsafe { &mut *track };
    free_c_string(&mut track.name);
    free_c_string(&mut track.source);
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub extern "C" fn kuroko_presenter_create() -> *mut KurokoPresenterHandle {
    create_presenter_handle(PresenterConfig::default())
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub extern "C" fn kuroko_presenter_create() -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub extern "C" fn kuroko_presenter_create_with_config(
    config: KurokoPresenterConfig,
) -> *mut KurokoPresenterHandle {
    create_presenter_handle(presenter_config_from_c(config))
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub extern "C" fn kuroko_presenter_create_with_output_mode(
    output_mode: i32,
    edr_headroom: f32,
) -> *mut KurokoPresenterHandle {
    create_presenter_handle(presenter_config_from_c(KurokoPresenterConfig {
        output_mode,
        edr_headroom,
    }))
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub extern "C" fn kuroko_presenter_create_with_config(
    _config: KurokoPresenterConfig,
) -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub extern "C" fn kuroko_presenter_create_with_output_mode(
    _output_mode: i32,
    _edr_headroom: f32,
) -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn create_presenter_handle(config: PresenterConfig) -> *mut KurokoPresenterHandle {
    match PresenterRuntime::new(config) {
        Ok(presenter) => {
            let events = presenter.player().subscribe();
            Box::into_raw(Box::new(KurokoPresenterHandle { presenter, events }))
        }
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn presenter_config_from_c(config: KurokoPresenterConfig) -> PresenterConfig {
    let output_mode = match KurokoPresenterOutputMode::from_raw(config.output_mode) {
        KurokoPresenterOutputMode::AppleEdr => {
            let headroom = if config.edr_headroom.is_finite() {
                config.edr_headroom
            } else {
                1.0
            };
            MetalOutputMode::apple_edr(headroom)
        }
        KurokoPresenterOutputMode::Sdr => MetalOutputMode::Sdr,
    };

    PresenterConfig {
        renderer: MetalRendererConfig { output_mode },
        ..PresenterConfig::default()
    }
}

fn danmaku_config_from_c(
    config: KurokoDanmakuConfig,
    base: &DanmakuLayoutConfig,
) -> DanmakuLayoutConfig {
    DanmakuLayoutConfig {
        enabled: config.enabled,
        font_size: config.font_size,
        opacity: config.opacity,
        display_area: config.display_area,
        scroll_duration_seconds: config.scroll_duration_seconds,
        scroll_speed_factor: config.scroll_speed_factor,
        track_gap_ratio: config.track_gap_ratio,
        outline_width: config.outline_width,
        shadow_offset: [config.shadow_offset_x, config.shadow_offset_y],
        merge_duplicates: config.merge_duplicates,
        allow_stacking: config.allow_stacking,
        allow_scroll_overwrite: config.allow_scroll_overwrite,
        max_quantity: (config.max_quantity > 0).then_some(config.max_quantity),
        max_lines_per_mode: (config.max_lines_per_mode > 0).then_some(config.max_lines_per_mode),
        block_top: config.block_top,
        block_bottom: config.block_bottom,
        block_scroll: config.block_scroll,
        block_words: base.block_words.clone(),
        shadow_style: DanmakuShadowStyle::from_code(config.shadow_style),
        custom_font_family: base.custom_font_family.clone(),
        custom_font_file_path: base.custom_font_file_path.clone(),
    }
}

fn danmaku_config_to_c(config: &DanmakuLayoutConfig) -> KurokoDanmakuConfig {
    KurokoDanmakuConfig {
        enabled: config.enabled,
        font_size: config.font_size,
        opacity: config.opacity,
        display_area: config.display_area,
        scroll_duration_seconds: config.scroll_duration_seconds,
        scroll_speed_factor: config.scroll_speed_factor,
        track_gap_ratio: config.track_gap_ratio,
        outline_width: config.outline_width,
        shadow_offset_x: config.shadow_offset[0],
        shadow_offset_y: config.shadow_offset[1],
        merge_duplicates: config.merge_duplicates,
        allow_stacking: config.allow_stacking,
        allow_scroll_overwrite: config.allow_scroll_overwrite,
        max_quantity: config.max_quantity.unwrap_or(0),
        max_lines_per_mode: config.max_lines_per_mode.unwrap_or(0),
        block_top: config.block_top,
        block_bottom: config.block_bottom,
        block_scroll: config.block_scroll,
        shadow_style: config.shadow_style.code(),
    }
}

fn danmaku_block_words_from_json(json: &str) -> Result<Vec<String>, KurokoStatus> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| KurokoStatus::PlayerError)?;
    match value {
        serde_json::Value::Array(items) => items
            .into_iter()
            .map(|item| match item {
                serde_json::Value::String(value) => Ok(value),
                _ => Err(KurokoStatus::PlayerError),
            })
            .collect(),
        serde_json::Value::String(value) => Ok(vec![value]),
        _ => Err(KurokoStatus::PlayerError),
    }
}

#[cfg(all(any(target_os = "macos", target_os = "ios"), test))]
fn metal_output_mode_from_c(config: KurokoPresenterConfig) -> MetalOutputMode {
    presenter_config_from_c(config).renderer.output_mode
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_destroy(handle: *mut KurokoPresenterHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_destroy(_handle: *mut std::ffi::c_void) {}

#[cfg(any(target_os = "macos", target_os = "ios"))]
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_play(handle: *mut KurokoPresenterHandle) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.play())
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_pause(
    handle: *mut KurokoPresenterHandle,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.pause())
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_stop(handle: *mut KurokoPresenterHandle) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.stop())
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_close(
    handle: *mut KurokoPresenterHandle,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.close())
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_playback_rate(
    handle: *mut KurokoPresenterHandle,
    rate: f64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.set_playback_rate(rate))
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_volume(
    handle: *mut KurokoPresenterHandle,
    volume: f64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        handle.presenter.set_volume(volume);
        KurokoStatus::Ok
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_add_external_subtitle(
    handle: *mut KurokoPresenterHandle,
    uri: *const c_char,
    out_track_id: *mut i64,
) -> KurokoStatus {
    if out_track_id.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_presenter_mut(handle, |handle| {
        let uri = match c_string(uri) {
            Ok(uri) => uri,
            Err(status) => return status,
        };
        match handle.presenter.add_external_subtitle(uri) {
            Ok(track) => {
                unsafe { *out_track_id = track.id };
                KurokoStatus::Ok
            }
            Err(_) => KurokoStatus::PlayerError,
        }
    })
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_add_external_subtitle(
    _handle: *mut std::ffi::c_void,
    _uri: *const c_char,
    _out_track_id: *mut i64,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_remove_subtitle_track(
    handle: *mut KurokoPresenterHandle,
    track_id: i64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.remove_subtitle_track(track_id))
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_select_audio_track(
    handle: *mut KurokoPresenterHandle,
    track_id: i64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(
            handle
                .presenter
                .select_audio_track(track_id_option(track_id)),
        )
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_select_subtitle_track(
    handle: *mut KurokoPresenterHandle,
    track_id: i64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(
            handle
                .presenter
                .select_subtitle_track(track_id_option(track_id)),
        )
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_load_danmaku_file(
    handle: *mut KurokoPresenterHandle,
    uri: *const c_char,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        let uri = match c_string(uri) {
            Ok(uri) => uri,
            Err(status) => return status,
        };
        match DanmakuTimeline::from_file(uri) {
            Ok(timeline) => {
                handle.presenter.set_danmaku_timeline(timeline);
                KurokoStatus::Ok
            }
            Err(_) => KurokoStatus::PlayerError,
        }
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_load_danmaku_json(
    handle: *mut KurokoPresenterHandle,
    json: *const c_char,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        let json = match c_string(json) {
            Ok(json) => json,
            Err(status) => return status,
        };
        match DanmakuTimeline::parse_auto(&json) {
            Ok(timeline) => {
                handle.presenter.set_danmaku_timeline(timeline);
                KurokoStatus::Ok
            }
            Err(_) => KurokoStatus::PlayerError,
        }
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_add_danmaku_track_file(
    handle: *mut KurokoPresenterHandle,
    uri: *const c_char,
    name: *const c_char,
    offset_micros: i64,
    out_track_id: *mut u64,
) -> KurokoStatus {
    if out_track_id.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_presenter_mut(handle, |handle| {
        let uri = match c_string(uri) {
            Ok(uri) => uri,
            Err(status) => return status,
        };
        let name = optional_c_string(name).unwrap_or_else(|| danmaku_track_name_from_uri(&uri));
        match DanmakuTimeline::from_file(&uri) {
            Ok(timeline) => {
                let track_id = handle.presenter.add_danmaku_track(
                    timeline,
                    name,
                    DanmakuTrackSource::File(std::path::PathBuf::from(uri)),
                    offset_micros,
                );
                unsafe { *out_track_id = track_id };
                KurokoStatus::Ok
            }
            Err(_) => KurokoStatus::PlayerError,
        }
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_add_danmaku_track_json(
    handle: *mut KurokoPresenterHandle,
    json: *const c_char,
    name: *const c_char,
    offset_micros: i64,
    out_track_id: *mut u64,
) -> KurokoStatus {
    if out_track_id.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_presenter_mut(handle, |handle| {
        let json = match c_string(json) {
            Ok(json) => json,
            Err(status) => return status,
        };
        let name = optional_c_string(name).unwrap_or_else(|| "danmaku".to_string());
        match DanmakuTimeline::parse_auto(&json) {
            Ok(timeline) => {
                let track_id = handle.presenter.add_danmaku_track(
                    timeline,
                    name,
                    DanmakuTrackSource::Json,
                    offset_micros,
                );
                unsafe { *out_track_id = track_id };
                KurokoStatus::Ok
            }
            Err(_) => KurokoStatus::PlayerError,
        }
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_remove_danmaku_track(
    handle: *mut KurokoPresenterHandle,
    track_id: u64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        if handle.presenter.remove_danmaku_track(track_id) {
            KurokoStatus::Ok
        } else {
            KurokoStatus::PlayerError
        }
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_track_enabled(
    handle: *mut KurokoPresenterHandle,
    track_id: u64,
    enabled: bool,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        if handle
            .presenter
            .set_danmaku_track_enabled(track_id, enabled)
        {
            KurokoStatus::Ok
        } else {
            KurokoStatus::PlayerError
        }
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_track_offset(
    handle: *mut KurokoPresenterHandle,
    track_id: u64,
    offset_micros: i64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        if handle
            .presenter
            .set_danmaku_track_offset(track_id, offset_micros)
        {
            KurokoStatus::Ok
        } else {
            KurokoStatus::PlayerError
        }
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_global_offset(
    handle: *mut KurokoPresenterHandle,
    offset_micros: i64,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        handle.presenter.set_danmaku_global_offset(offset_micros);
        KurokoStatus::Ok
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_danmaku_tracks(
    handle: *mut KurokoPresenterHandle,
    out_tracks: *mut KurokoDanmakuTrackInfo,
    capacity: usize,
    out_len: *mut usize,
) -> KurokoStatus {
    if out_len.is_null() || (capacity > 0 && out_tracks.is_null()) {
        return KurokoStatus::NullPointer;
    }
    with_presenter_mut(handle, |handle| {
        write_danmaku_tracks_to_c(
            &handle.presenter.danmaku_tracks(),
            out_tracks,
            capacity,
            out_len,
        )
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_clear_danmaku(
    handle: *mut KurokoPresenterHandle,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        handle.presenter.clear_danmaku();
        KurokoStatus::Ok
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_enabled(
    handle: *mut KurokoPresenterHandle,
    enabled: bool,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        handle.presenter.set_danmaku_enabled(enabled);
        KurokoStatus::Ok
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_config(
    handle: *mut KurokoPresenterHandle,
    config: KurokoDanmakuConfig,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        let base = handle
            .presenter
            .danmaku_config()
            .cloned()
            .unwrap_or_default();
        handle
            .presenter
            .set_danmaku_config(danmaku_config_from_c(config, &base));
        KurokoStatus::Ok
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_config_ptr(
    handle: *mut KurokoPresenterHandle,
    config: *const KurokoDanmakuConfig,
) -> KurokoStatus {
    if config.is_null() {
        return KurokoStatus::NullPointer;
    }
    unsafe { kuroko_presenter_set_danmaku_config(handle, *config) }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_get_danmaku_config(
    handle: *mut KurokoPresenterHandle,
    out_config: *mut KurokoDanmakuConfig,
) -> KurokoStatus {
    if out_config.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_presenter_mut(handle, |handle| {
        let config = handle
            .presenter
            .danmaku_config()
            .map(danmaku_config_to_c)
            .unwrap_or_default();
        unsafe {
            *out_config = config;
        }
        KurokoStatus::Ok
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_font(
    handle: *mut KurokoPresenterHandle,
    family: *const c_char,
    file_path: *const c_char,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        let family = optional_c_string(family).unwrap_or_default();
        let file_path = optional_c_string(file_path).unwrap_or_default();
        handle.presenter.set_danmaku_font(family, file_path);
        KurokoStatus::Ok
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_block_words_json(
    handle: *mut KurokoPresenterHandle,
    json: *const c_char,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        let json = match c_string(json) {
            Ok(json) => json,
            Err(status) => return status,
        };
        let block_words = match danmaku_block_words_from_json(&json) {
            Ok(words) => words,
            Err(status) => return status,
        };
        let mut config = handle
            .presenter
            .danmaku_config()
            .cloned()
            .unwrap_or_default();
        config.block_words = block_words;
        handle.presenter.set_danmaku_config(config);
        KurokoStatus::Ok
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_track_selection(
    handle: *mut KurokoPresenterHandle,
    out_selection: *mut KurokoTrackSelection,
) -> KurokoStatus {
    if out_selection.is_null() {
        return KurokoStatus::NullPointer;
    }
    with_presenter_mut(handle, |handle| {
        unsafe { *out_selection = track_selection_to_c(handle.presenter.track_selection()) };
        KurokoStatus::Ok
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_tracks(
    handle: *mut KurokoPresenterHandle,
    out_tracks: *mut KurokoTrackInfo,
    capacity: usize,
    out_len: *mut usize,
) -> KurokoStatus {
    if out_len.is_null() || (capacity > 0 && out_tracks.is_null()) {
        return KurokoStatus::NullPointer;
    }
    with_presenter_mut(handle, |handle| {
        write_tracks_to_c(&handle.presenter.tracks(), out_tracks, capacity, out_len)
    })
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_remove_subtitle_track(
    _handle: *mut std::ffi::c_void,
    _track_id: i64,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_select_audio_track(
    _handle: *mut std::ffi::c_void,
    _track_id: i64,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_select_subtitle_track(
    _handle: *mut std::ffi::c_void,
    _track_id: i64,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_track_selection(
    _handle: *mut std::ffi::c_void,
    _out_selection: *mut KurokoTrackSelection,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_tracks(
    _handle: *mut std::ffi::c_void,
    _out_tracks: *mut KurokoTrackInfo,
    _capacity: usize,
    _out_len: *mut usize,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_playback_rate(
    _handle: *mut std::ffi::c_void,
    _rate: f64,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_volume(
    _handle: *mut std::ffi::c_void,
    _volume: f64,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_load_danmaku_file(
    _handle: *mut std::ffi::c_void,
    _uri: *const c_char,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_load_danmaku_json(
    _handle: *mut std::ffi::c_void,
    _json: *const c_char,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_add_danmaku_track_file(
    _handle: *mut std::ffi::c_void,
    uri: *const c_char,
    _name: *const c_char,
    _offset_micros: i64,
    out_track_id: *mut u64,
) -> KurokoStatus {
    if uri.is_null() || out_track_id.is_null() {
        return KurokoStatus::NullPointer;
    }
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_add_danmaku_track_json(
    _handle: *mut std::ffi::c_void,
    json: *const c_char,
    _name: *const c_char,
    _offset_micros: i64,
    out_track_id: *mut u64,
) -> KurokoStatus {
    if json.is_null() || out_track_id.is_null() {
        return KurokoStatus::NullPointer;
    }
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_remove_danmaku_track(
    _handle: *mut std::ffi::c_void,
    _track_id: u64,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_track_enabled(
    _handle: *mut std::ffi::c_void,
    _track_id: u64,
    _enabled: bool,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_track_offset(
    _handle: *mut std::ffi::c_void,
    _track_id: u64,
    _offset_micros: i64,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_global_offset(
    _handle: *mut std::ffi::c_void,
    _offset_micros: i64,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_danmaku_tracks(
    _handle: *mut std::ffi::c_void,
    out_tracks: *mut KurokoDanmakuTrackInfo,
    capacity: usize,
    out_len: *mut usize,
) -> KurokoStatus {
    if out_len.is_null() || (capacity > 0 && out_tracks.is_null()) {
        return KurokoStatus::NullPointer;
    }
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_clear_danmaku(
    _handle: *mut std::ffi::c_void,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_enabled(
    _handle: *mut std::ffi::c_void,
    _enabled: bool,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_config(
    _handle: *mut std::ffi::c_void,
    _config: KurokoDanmakuConfig,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_config_ptr(
    _handle: *mut std::ffi::c_void,
    config: *const KurokoDanmakuConfig,
) -> KurokoStatus {
    if config.is_null() {
        return KurokoStatus::NullPointer;
    }
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_get_danmaku_config(
    _handle: *mut std::ffi::c_void,
    out_config: *mut KurokoDanmakuConfig,
) -> KurokoStatus {
    if out_config.is_null() {
        return KurokoStatus::NullPointer;
    }
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_font(
    _handle: *mut std::ffi::c_void,
    _family: *const c_char,
    _file_path: *const c_char,
) -> KurokoStatus {
    KurokoStatus::PlayerError
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_set_danmaku_block_words_json(
    _handle: *mut std::ffi::c_void,
    json: *const c_char,
) -> KurokoStatus {
    if json.is_null() {
        return KurokoStatus::NullPointer;
    }
    KurokoStatus::PlayerError
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuroko_presenter_detach_surface(
    handle: *mut KurokoPresenterHandle,
) -> KurokoStatus {
    with_presenter_mut(handle, |handle| {
        status_from_player_result(handle.presenter.detach_surface())
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
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

fn optional_c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let value = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()?
        .trim()
        .to_string();
    (!value.is_empty()).then_some(value)
}

fn status_from_player_result(result: kuroko::Result<()>) -> KurokoStatus {
    match result {
        Ok(()) => KurokoStatus::Ok,
        Err(_) => KurokoStatus::PlayerError,
    }
}

fn track_id_option(track_id: i64) -> Option<i64> {
    (track_id >= 0).then_some(track_id)
}

fn write_tracks_to_c(
    tracks: &[TrackInfo],
    out_tracks: *mut KurokoTrackInfo,
    capacity: usize,
    out_len: *mut usize,
) -> KurokoStatus {
    unsafe { *out_len = tracks.len() };
    if capacity == 0 {
        return KurokoStatus::Ok;
    }
    let count = tracks.len().min(capacity);
    for (index, track) in tracks.iter().take(count).enumerate() {
        unsafe { *out_tracks.add(index) = track_info_to_c(track) };
    }
    KurokoStatus::Ok
}

fn write_danmaku_tracks_to_c(
    tracks: &[DanmakuTrackInfo],
    out_tracks: *mut KurokoDanmakuTrackInfo,
    capacity: usize,
    out_len: *mut usize,
) -> KurokoStatus {
    unsafe { *out_len = tracks.len() };
    if capacity == 0 {
        return KurokoStatus::Ok;
    }
    let count = tracks.len().min(capacity);
    for (index, track) in tracks.iter().take(count).enumerate() {
        unsafe { *out_tracks.add(index) = danmaku_track_info_to_c(track) };
    }
    KurokoStatus::Ok
}

fn danmaku_track_info_to_c(track: &DanmakuTrackInfo) -> KurokoDanmakuTrackInfo {
    KurokoDanmakuTrackInfo {
        id: track.id,
        enabled: track.enabled,
        offset_micros: track.offset_micros,
        item_count: track.item_count,
        name: option_string_to_c(Some(&track.name)),
        source: option_string_to_c(Some(&track.source)),
    }
}

fn danmaku_track_name_from_uri(uri: &str) -> String {
    std::path::Path::new(uri)
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("danmaku")
        .to_string()
}

fn track_info_to_c(track: &TrackInfo) -> KurokoTrackInfo {
    KurokoTrackInfo {
        id: track.id,
        kind: track_kind_to_c(track.kind),
        source: track_source_to_c(track.source),
        selected: track.selected,
        can_remove: track.can_remove,
        title: option_string_to_c(track.title.as_deref()),
        language: option_string_to_c(track.language.as_deref()),
        codec: option_string_to_c(track.codec.as_deref()),
    }
}

fn track_kind_to_c(kind: TrackKind) -> KurokoTrackKind {
    match kind {
        TrackKind::Video => KurokoTrackKind::Video,
        TrackKind::Audio => KurokoTrackKind::Audio,
        TrackKind::Subtitle => KurokoTrackKind::Subtitle,
    }
}

fn track_source_to_c(source: TrackSource) -> KurokoTrackSource {
    match source {
        TrackSource::Embedded => KurokoTrackSource::Embedded,
        TrackSource::External => KurokoTrackSource::External,
    }
}

fn track_selection_to_c(selection: TrackSelection) -> KurokoTrackSelection {
    KurokoTrackSelection {
        video: selection.video.unwrap_or(-1),
        audio: selection.audio.unwrap_or(-1),
        subtitle: selection.subtitle.unwrap_or(-1),
    }
}

fn option_string_to_c(value: Option<&str>) -> *mut c_char {
    let Some(value) = value else {
        return std::ptr::null_mut();
    };
    match CString::new(value) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn free_c_string(ptr: &mut *mut c_char) {
    if ptr.is_null() {
        return;
    }
    let raw = *ptr;
    *ptr = std::ptr::null_mut();
    unsafe { drop(CString::from_raw(raw)) };
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
        PlayerEvent::TrackSelectionChanged(_) => KurokoEvent {
            kind: KurokoEventKind::TrackSelectionChanged,
            ..KurokoEvent::default()
        },
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn presenter_stats_to_c(stats: PresenterStats) -> KurokoPresenterStats {
    KurokoPresenterStats {
        decoded_video_frames: stats.decoded_video_frames,
        rendered_video_frames: stats.rendered_video_frames,
        rendered_test_frames: stats.rendered_test_frames,
        pushed_audio_frames: stats.pushed_audio_frames,
        overlay_frames: stats.overlay_frames,
        danmaku_frames: stats.danmaku_frames,
        danmaku_items: stats.danmaku_items,
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
            kuroko::TrackInfo::embedded(0, TrackKind::Video),
            kuroko::TrackInfo::embedded(1, TrackKind::Audio),
        ]));

        assert_eq!(event.kind, KurokoEventKind::TracksChanged);
        assert_eq!(event.tracks.video, 1);
        assert_eq!(event.tracks.audio, 1);
        assert_eq!(event.tracks.subtitle, 0);
    }

    #[test]
    fn c_event_reports_track_selection_changed() {
        let event = event_to_c(PlayerEvent::TrackSelectionChanged(kuroko::TrackSelection {
            video: Some(0),
            audio: Some(1),
            subtitle: None,
        }));

        assert_eq!(event.kind, KurokoEventKind::TrackSelectionChanged);
    }

    #[test]
    fn c_track_info_maps_source_selection_and_strings() {
        let mut track = kuroko::TrackInfo::external(1_000_001, TrackKind::Subtitle);
        track.selected = true;
        track.title = Some("Signs".to_string());
        track.language = Some("jpn".to_string());
        track.codec = Some("ass".to_string());

        let mut c_track = track_info_to_c(&track);

        assert_eq!(c_track.id, 1_000_001);
        assert_eq!(c_track.kind, KurokoTrackKind::Subtitle);
        assert_eq!(c_track.source, KurokoTrackSource::External);
        assert!(c_track.selected);
        assert!(c_track.can_remove);
        assert_eq!(
            unsafe { CStr::from_ptr(c_track.title).to_str().unwrap() },
            "Signs"
        );
        assert_eq!(
            unsafe { CStr::from_ptr(c_track.language).to_str().unwrap() },
            "jpn"
        );
        assert_eq!(
            unsafe { CStr::from_ptr(c_track.codec).to_str().unwrap() },
            "ass"
        );

        unsafe { kuroko_track_info_free(&mut c_track) };
        assert!(c_track.title.is_null());
        assert!(c_track.language.is_null());
        assert!(c_track.codec.is_null());
    }

    #[test]
    fn c_track_selection_uses_negative_one_for_disabled_tracks() {
        let selection = track_selection_to_c(kuroko::TrackSelection {
            video: Some(0),
            audio: None,
            subtitle: Some(2),
        });

        assert_eq!(selection.video, 0);
        assert_eq!(selection.audio, -1);
        assert_eq!(selection.subtitle, 2);
    }

    #[test]
    fn null_handle_is_rejected() {
        assert_eq!(
            unsafe { kuroko_play(std::ptr::null_mut()) },
            KurokoStatus::NullPointer
        );
    }

    #[test]
    fn c_external_subtitle_api_rejects_null_pointers() {
        let subtitle_uri = std::ffi::CString::new("/tmp/subs.srt").unwrap();
        let handle = kuroko_create();
        assert!(!handle.is_null());

        let status = unsafe {
            kuroko_add_external_subtitle(handle, subtitle_uri.as_ptr(), std::ptr::null_mut())
        };
        assert_eq!(status, KurokoStatus::NullPointer);

        let mut track_id = 0;
        let status = unsafe {
            kuroko_add_external_subtitle(std::ptr::null_mut(), subtitle_uri.as_ptr(), &mut track_id)
        };
        assert_eq!(status, KurokoStatus::NullPointer);

        let status = unsafe { kuroko_remove_subtitle_track(std::ptr::null_mut(), 1_000_001) };
        assert_eq!(status, KurokoStatus::NullPointer);

        unsafe { kuroko_destroy(handle) };
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

    #[cfg(any(target_os = "macos", target_os = "ios"))]
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

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn c_presenter_set_volume_accepts_valid_handle() {
        assert_eq!(
            unsafe { kuroko_presenter_set_volume(std::ptr::null_mut(), 0.5) },
            KurokoStatus::NullPointer
        );

        let handle = kuroko_presenter_create();
        assert!(!handle.is_null());
        assert_eq!(
            unsafe { kuroko_presenter_set_volume(handle, 0.5) },
            KurokoStatus::Ok
        );
        assert_eq!(
            unsafe { kuroko_presenter_set_volume(handle, f64::NAN) },
            KurokoStatus::Ok
        );
        unsafe { kuroko_presenter_destroy(handle) };
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn c_presenter_can_be_created_with_edr_config() {
        let handle = kuroko_presenter_create_with_config(KurokoPresenterConfig {
            output_mode: KurokoPresenterOutputMode::AppleEdr as i32,
            edr_headroom: 4.0,
        });
        assert!(!handle.is_null());
        unsafe { kuroko_presenter_destroy(handle) };
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn c_presenter_danmaku_api_loads_configures_and_clears() {
        let handle = kuroko_presenter_create();
        assert!(!handle.is_null());

        let json = CString::new(
            r##"{"comments":[{"time":1.0,"content":"c api danmaku","type":"scroll","color":"#ffffff"}]}"##,
        )
        .unwrap();
        assert_eq!(
            unsafe { kuroko_presenter_load_danmaku_json(handle, json.as_ptr()) },
            KurokoStatus::Ok
        );
        assert_eq!(
            unsafe { kuroko_presenter_set_danmaku_enabled(handle, true) },
            KurokoStatus::Ok
        );
        assert_eq!(
            unsafe { kuroko_presenter_set_danmaku_config(handle, KurokoDanmakuConfig::default()) },
            KurokoStatus::Ok
        );
        assert_eq!(
            unsafe { kuroko_presenter_clear_danmaku(handle) },
            KurokoStatus::Ok
        );

        unsafe { kuroko_presenter_destroy(handle) };
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn c_presenter_config_maps_output_modes() {
        assert_eq!(
            metal_output_mode_from_c(KurokoPresenterConfig::default()),
            MetalOutputMode::Sdr
        );
        assert_eq!(
            metal_output_mode_from_c(KurokoPresenterConfig {
                output_mode: KurokoPresenterOutputMode::AppleEdr as i32,
                edr_headroom: 4.0,
            }),
            MetalOutputMode::apple_edr(4.0)
        );
        assert_eq!(
            metal_output_mode_from_c(KurokoPresenterConfig {
                output_mode: 999,
                edr_headroom: 4.0,
            }),
            MetalOutputMode::Sdr
        );
        assert_eq!(
            metal_output_mode_from_c(KurokoPresenterConfig {
                output_mode: KurokoPresenterOutputMode::AppleEdr as i32,
                edr_headroom: f32::NAN,
            }),
            MetalOutputMode::apple_edr(1.0)
        );
    }
}
