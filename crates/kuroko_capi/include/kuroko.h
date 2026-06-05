#ifndef KUROKO_H
#define KUROKO_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct KurokoHandle KurokoHandle;
typedef struct KurokoPresenterHandle KurokoPresenterHandle;

typedef enum KurokoStatus {
  KurokoStatus_Ok = 0,
  KurokoStatus_NullPointer = 1,
  KurokoStatus_InvalidUtf8 = 2,
  KurokoStatus_PlayerError = 3,
  KurokoStatus_Panic = 4,
  KurokoStatus_NoEvent = 5,
} KurokoStatus;

typedef enum KurokoState {
  KurokoState_Idle = 0,
  KurokoState_Opening = 1,
  KurokoState_Ready = 2,
  KurokoState_Playing = 3,
  KurokoState_Paused = 4,
  KurokoState_Stopped = 5,
  KurokoState_Closed = 6,
  KurokoState_Error = 7,
} KurokoState;

typedef enum KurokoEventKind {
  KurokoEventKind_None = 0,
  KurokoEventKind_StateChanged = 1,
  KurokoEventKind_DurationChanged = 2,
  KurokoEventKind_PositionChanged = 3,
  KurokoEventKind_TracksChanged = 4,
  KurokoEventKind_BufferingChanged = 5,
  KurokoEventKind_VideoParamsChanged = 6,
  KurokoEventKind_SurfaceAttached = 7,
  KurokoEventKind_SurfaceDetached = 8,
  KurokoEventKind_Error = 9,
  KurokoEventKind_TrackSelectionChanged = 10,
} KurokoEventKind;

typedef enum KurokoTrackKind {
  KurokoTrackKind_Video = 0,
  KurokoTrackKind_Audio = 1,
  KurokoTrackKind_Subtitle = 2,
} KurokoTrackKind;

typedef enum KurokoTrackSource {
  KurokoTrackSource_Embedded = 0,
  KurokoTrackSource_External = 1,
} KurokoTrackSource;

typedef enum KurokoWgpuSurfaceKind {
  KurokoWgpuSurfaceKind_Unknown = 0,
  KurokoWgpuSurfaceKind_MacOsNsView = 1,
  KurokoWgpuSurfaceKind_MacOsCaMetalLayer = 2,
  KurokoWgpuSurfaceKind_IosUiView = 3,
  KurokoWgpuSurfaceKind_WindowsHwnd = 4,
  KurokoWgpuSurfaceKind_XlibWindow = 5,
  KurokoWgpuSurfaceKind_WaylandSurface = 6,
  KurokoWgpuSurfaceKind_AndroidNativeWindow = 7,
} KurokoWgpuSurfaceKind;

typedef enum KurokoFlutterTextureKind {
  KurokoFlutterTextureKind_Unknown = 0,
  KurokoFlutterTextureKind_MacOsTextureRegistrar = 1,
  KurokoFlutterTextureKind_IosTextureRegistrar = 2,
  KurokoFlutterTextureKind_AndroidSurfaceTexture = 3,
  KurokoFlutterTextureKind_WindowsTextureRegistrar = 4,
  KurokoFlutterTextureKind_LinuxTextureRegistrar = 5,
} KurokoFlutterTextureKind;

typedef enum KurokoPresenterOutputMode {
  KurokoPresenterOutputMode_Sdr = 0,
  KurokoPresenterOutputMode_AppleEdr = 1,
} KurokoPresenterOutputMode;

typedef struct KurokoPresenterConfig {
  int32_t output_mode;
  float edr_headroom;
} KurokoPresenterConfig;

typedef struct KurokoDanmakuConfig {
  bool enabled;
  float font_size;
  float opacity;
  float display_area;
  float scroll_duration_seconds;
  float scroll_speed_factor;
  float track_gap_ratio;
  float outline_width;
  float shadow_offset_x;
  float shadow_offset_y;
  bool merge_duplicates;
  bool allow_stacking;
  bool allow_scroll_overwrite;
  uint32_t max_quantity;
  uint32_t max_lines_per_mode;
  bool block_top;
  bool block_bottom;
  bool block_scroll;
  int32_t shadow_style;
} KurokoDanmakuConfig;

typedef struct KurokoDanmakuTrackInfo {
  uint64_t id;
  bool enabled;
  int64_t offset_micros;
  uintptr_t item_count;
  char *name;
  char *source;
} KurokoDanmakuTrackInfo;

typedef struct KurokoVideoParams {
  uint32_t width;
  uint32_t height;
  uint32_t primaries;
  uint32_t transfer;
} KurokoVideoParams;

typedef struct KurokoTrackCounts {
  uint32_t video;
  uint32_t audio;
  uint32_t subtitle;
} KurokoTrackCounts;

typedef struct KurokoTrackSelection {
  int64_t video;
  int64_t audio;
  int64_t subtitle;
} KurokoTrackSelection;

typedef struct KurokoTrackInfo {
  int64_t id;
  KurokoTrackKind kind;
  KurokoTrackSource source;
  bool selected;
  bool can_remove;
  char *title;
  char *language;
  char *codec;
} KurokoTrackInfo;

typedef struct KurokoEvent {
  KurokoEventKind kind;
  KurokoStatus status;
  KurokoState state;
  int64_t duration_micros;
  uint64_t position_micros;
  bool buffering;
  KurokoVideoParams video;
  KurokoTrackCounts tracks;
} KurokoEvent;

typedef struct KurokoPresenterStats {
  uint64_t decoded_video_frames;
  uint64_t rendered_video_frames;
  uint64_t rendered_test_frames;
  uint64_t pushed_audio_frames;
  uint64_t overlay_frames;
  uint64_t danmaku_frames;
  uint64_t danmaku_items;
  uint64_t import_failures;
  uint64_t render_failures;
  uint64_t audio_failures;
} KurokoPresenterStats;

KurokoHandle *kuroko_create(void);
void kuroko_destroy(KurokoHandle *handle);

KurokoStatus kuroko_open(KurokoHandle *handle, const char *uri);
KurokoStatus kuroko_play(KurokoHandle *handle);
KurokoStatus kuroko_pause(KurokoHandle *handle);
KurokoStatus kuroko_stop(KurokoHandle *handle);
KurokoStatus kuroko_close(KurokoHandle *handle);
KurokoStatus kuroko_seek(KurokoHandle *handle, uint64_t position_micros);
KurokoStatus kuroko_add_external_subtitle(
    KurokoHandle *handle,
    const char *uri,
    int64_t *out_track_id);
KurokoStatus kuroko_remove_subtitle_track(KurokoHandle *handle, int64_t track_id);
KurokoStatus kuroko_select_audio_track(KurokoHandle *handle, int64_t track_id);
KurokoStatus kuroko_select_subtitle_track(KurokoHandle *handle, int64_t track_id);
KurokoStatus kuroko_track_selection(
    KurokoHandle *handle,
    KurokoTrackSelection *out_selection);
KurokoStatus kuroko_tracks(
    KurokoHandle *handle,
    KurokoTrackInfo *out_tracks,
    uintptr_t capacity,
    uintptr_t *out_len);
void kuroko_track_info_free(KurokoTrackInfo *track);
void kuroko_danmaku_track_info_free(KurokoDanmakuTrackInfo *track);
KurokoStatus kuroko_state(KurokoHandle *handle, KurokoState *out_state);
KurokoStatus kuroko_poll_event(KurokoHandle *handle, KurokoEvent *out_event);

KurokoStatus kuroko_attach_metal_layer(
    KurokoHandle *handle,
    uint64_t raw_layer,
    uint32_t width,
    uint32_t height,
    double scale);

KurokoStatus kuroko_attach_wgpu_surface(
    KurokoHandle *handle,
    KurokoWgpuSurfaceKind kind,
    uint64_t raw_window,
    uint64_t raw_display,
    uint32_t width,
    uint32_t height,
    double scale);

KurokoStatus kuroko_attach_flutter_texture(
    KurokoHandle *handle,
    KurokoFlutterTextureKind kind,
    int64_t texture_id,
    uint32_t width,
    uint32_t height,
    double scale);

KurokoStatus kuroko_detach_surface(KurokoHandle *handle);

KurokoPresenterHandle *kuroko_presenter_create(void);
KurokoPresenterHandle *kuroko_presenter_create_with_config(KurokoPresenterConfig config);
KurokoPresenterHandle *kuroko_presenter_create_with_output_mode(
    int32_t output_mode,
    float edr_headroom);
void kuroko_presenter_destroy(KurokoPresenterHandle *handle);

KurokoStatus kuroko_presenter_open(KurokoPresenterHandle *handle, const char *uri);
KurokoStatus kuroko_presenter_play(KurokoPresenterHandle *handle);
KurokoStatus kuroko_presenter_pause(KurokoPresenterHandle *handle);
KurokoStatus kuroko_presenter_stop(KurokoPresenterHandle *handle);
KurokoStatus kuroko_presenter_close(KurokoPresenterHandle *handle);
KurokoStatus kuroko_presenter_seek(KurokoPresenterHandle *handle, uint64_t position_micros);
KurokoStatus kuroko_presenter_set_playback_rate(KurokoPresenterHandle *handle, double rate);
KurokoStatus kuroko_presenter_add_external_subtitle(
    KurokoPresenterHandle *handle,
    const char *uri,
    int64_t *out_track_id);
KurokoStatus kuroko_presenter_remove_subtitle_track(
    KurokoPresenterHandle *handle,
    int64_t track_id);
KurokoStatus kuroko_presenter_select_audio_track(
    KurokoPresenterHandle *handle,
    int64_t track_id);
KurokoStatus kuroko_presenter_select_subtitle_track(
    KurokoPresenterHandle *handle,
    int64_t track_id);
KurokoStatus kuroko_presenter_load_danmaku_file(
    KurokoPresenterHandle *handle,
    const char *uri);
KurokoStatus kuroko_presenter_load_danmaku_json(
    KurokoPresenterHandle *handle,
    const char *json);
KurokoStatus kuroko_presenter_add_danmaku_track_file(
    KurokoPresenterHandle *handle,
    const char *uri,
    const char *name,
    int64_t offset_micros,
    uint64_t *out_track_id);
KurokoStatus kuroko_presenter_add_danmaku_track_json(
    KurokoPresenterHandle *handle,
    const char *json,
    const char *name,
    int64_t offset_micros,
    uint64_t *out_track_id);
KurokoStatus kuroko_presenter_remove_danmaku_track(
    KurokoPresenterHandle *handle,
    uint64_t track_id);
KurokoStatus kuroko_presenter_set_danmaku_track_enabled(
    KurokoPresenterHandle *handle,
    uint64_t track_id,
    bool enabled);
KurokoStatus kuroko_presenter_set_danmaku_track_offset(
    KurokoPresenterHandle *handle,
    uint64_t track_id,
    int64_t offset_micros);
KurokoStatus kuroko_presenter_set_danmaku_global_offset(
    KurokoPresenterHandle *handle,
    int64_t offset_micros);
KurokoStatus kuroko_presenter_danmaku_tracks(
    KurokoPresenterHandle *handle,
    KurokoDanmakuTrackInfo *out_tracks,
    uintptr_t capacity,
    uintptr_t *out_len);
KurokoStatus kuroko_presenter_clear_danmaku(KurokoPresenterHandle *handle);
KurokoStatus kuroko_presenter_set_danmaku_enabled(
    KurokoPresenterHandle *handle,
    bool enabled);
KurokoStatus kuroko_presenter_set_danmaku_config(
    KurokoPresenterHandle *handle,
    KurokoDanmakuConfig config);
KurokoStatus kuroko_presenter_set_danmaku_config_ptr(
    KurokoPresenterHandle *handle,
    const KurokoDanmakuConfig *config);
KurokoStatus kuroko_presenter_set_danmaku_font(
    KurokoPresenterHandle *handle,
    const char *family,
    const char *file_path);
KurokoStatus kuroko_presenter_set_danmaku_block_words_json(
    KurokoPresenterHandle *handle,
    const char *json);
KurokoStatus kuroko_presenter_track_selection(
    KurokoPresenterHandle *handle,
    KurokoTrackSelection *out_selection);
KurokoStatus kuroko_presenter_tracks(
    KurokoPresenterHandle *handle,
    KurokoTrackInfo *out_tracks,
    uintptr_t capacity,
    uintptr_t *out_len);

KurokoStatus kuroko_presenter_attach_metal_layer(
    KurokoPresenterHandle *handle,
    uint64_t raw_layer,
    uint32_t width,
    uint32_t height,
    double scale);

KurokoStatus kuroko_presenter_resize_surface(
    KurokoPresenterHandle *handle,
    uint32_t width,
    uint32_t height,
    double scale);

KurokoStatus kuroko_presenter_detach_surface(KurokoPresenterHandle *handle);
KurokoStatus kuroko_presenter_render_tick(
    KurokoPresenterHandle *handle,
    double time_seconds,
    KurokoPresenterStats *out_stats);
KurokoStatus kuroko_presenter_poll_event(KurokoPresenterHandle *handle, KurokoEvent *out_event);

#ifdef __cplusplus
}
#endif

#endif
