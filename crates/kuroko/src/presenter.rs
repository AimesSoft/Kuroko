use std::time::Duration;

use crossbeam_channel::Receiver;

use crate::apple::coreaudio::{CoreAudioOutput, CoreAudioOutputConfig};
use crate::core::{
    MediaRequest, PlatformSurface, Player, PlayerAudioFrame, PlayerConfig, PlayerSubtitleFrame,
    PlayerVideoFrame, RendererBackend, RendererBackendPreference, TrackInfo, TrackSelection,
};
use crate::overlay::{OverlayFrame, OverlayTimeline, OverlayViewport};
use crate::renderer::metal::{MetalRenderer, MetalRendererConfig};
#[cfg(feature = "libass")]
use crate::subtitle::decoded_subtitle_frames_to_ass_script;
use crate::subtitle::{
    DecodedSubtitleFrame, SubtitleRendererCore, SubtitleTrackConfig, SubtitleViewport,
    decoded_subtitle_frames_to_timeline,
};
#[cfg(feature = "libass")]
use crate::subtitle::{
    LibassRenderConfig, LibassSubtitleRenderer, SubtitleRenderRequest, SubtitleRenderer,
};
use crate::{PlayerError, Result};

#[derive(Debug, Clone)]
pub struct PresenterConfig {
    pub player: PlayerConfig,
    pub audio: CoreAudioOutputConfig,
    pub renderer: MetalRendererConfig,
    pub overlay: OverlayTimeline,
}

impl Default for PresenterConfig {
    fn default() -> Self {
        Self {
            player: PlayerConfig::default(),
            audio: CoreAudioOutputConfig::default(),
            renderer: MetalRendererConfig::default(),
            overlay: OverlayTimeline::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresenterStats {
    pub decoded_video_frames: u64,
    pub rendered_video_frames: u64,
    pub rendered_test_frames: u64,
    pub pushed_audio_frames: u64,
    pub decoded_subtitle_frames: u64,
    pub overlay_frames: u64,
    pub import_failures: u64,
    pub render_failures: u64,
    pub audio_failures: u64,
}

pub struct PresenterRuntime {
    player: Player,
    renderer: Box<dyn RendererBackend>,
    video_frames: Receiver<PlayerVideoFrame>,
    audio_frames: Receiver<PlayerAudioFrame>,
    subtitle_frames: Receiver<PlayerSubtitleFrame>,
    audio_output: CoreAudioOutput,
    audio_started: bool,
    current_overlay: Option<OverlayFrame>,
    subtitles: SubtitleFrameState,
    overlay: OverlayTimeline,
    stats: PresenterStats,
}

impl PresenterRuntime {
    pub fn new(config: PresenterConfig) -> Result<Self> {
        let renderer = build_renderer(config.player.renderer, config.renderer)?;
        let player = Player::new(config.player);
        let video_frames = player.subscribe_video_frames();
        let audio_frames = player.subscribe_audio_frames();
        let subtitle_frames = player.subscribe_subtitle_frames();
        Ok(Self {
            player,
            renderer,
            video_frames,
            audio_frames,
            subtitle_frames,
            audio_output: CoreAudioOutput::new(config.audio),
            audio_started: false,
            current_overlay: None,
            subtitles: SubtitleFrameState::default(),
            overlay: config.overlay,
            stats: PresenterStats::default(),
        })
    }

    pub fn player(&self) -> &Player {
        &self.player
    }

    pub fn attach_surface(&mut self, surface: PlatformSurface) -> Result<()> {
        self.player.attach_surface(surface)?;
        self.renderer.attach_surface(surface)
    }

    pub fn detach_surface(&mut self) -> Result<()> {
        self.player.detach_surface()?;
        self.renderer.detach_surface()
    }

    pub fn resize_surface(&mut self, width: u32, height: u32, scale: f64) -> Result<()> {
        self.renderer.resize_surface(width, height, scale)
    }

    pub fn open(&self, media: MediaRequest) -> Result<()> {
        self.player.open(media)
    }

    pub fn play(&self) -> Result<()> {
        self.player.play()
    }

    pub fn pause(&self) -> Result<()> {
        self.player.pause()
    }

    pub fn stop(&self) -> Result<()> {
        self.player.stop()
    }

    pub fn close(&self) -> Result<()> {
        self.player.close()
    }

    pub fn seek(&self, position: Duration) -> Result<()> {
        self.player.seek(position)
    }

    pub fn add_external_subtitle(&self, uri: impl Into<String>) -> Result<SubtitleTrackConfig> {
        self.player.add_external_subtitle(uri)
    }

    pub fn remove_subtitle_track(&self, track_id: i64) -> Result<()> {
        self.player.remove_subtitle_track(track_id)
    }

    pub fn select_audio_track(&self, track_id: Option<i64>) -> Result<()> {
        self.player.select_audio_track(track_id)
    }

    pub fn select_subtitle_track(&self, track_id: Option<i64>) -> Result<()> {
        self.player.select_subtitle_track(track_id)
    }

    pub fn tracks(&self) -> Vec<TrackInfo> {
        self.player.tracks()
    }

    pub fn track_selection(&self) -> TrackSelection {
        self.player.track_selection()
    }

    pub fn render_tick(&mut self, time_seconds: f64) -> Result<PresenterStats> {
        self.pump_audio();
        self.pump_subtitles();
        self.pump_video();

        match self
            .renderer
            .render_current_frame(self.current_overlay.as_ref())
        {
            Ok(true) => self.stats.rendered_video_frames += 1,
            Ok(false) => {
                self.renderer.render_test_frame(time_seconds)?;
                self.stats.rendered_test_frames += 1;
            }
            Err(error) => {
                self.stats.render_failures += 1;
                return Err(error);
            }
        }

        Ok(self.stats)
    }

    pub fn stats(&self) -> PresenterStats {
        self.stats
    }

    fn pump_video(&mut self) {
        loop {
            match self.video_frames.try_recv() {
                Ok(frame) => {
                    self.stats.decoded_video_frames += 1;
                    match self.renderer.upload_player_frame(&frame) {
                        Ok(()) => {
                            let pts = frame.pts.unwrap_or(frame.media_time);
                            self.update_overlay(
                                pts,
                                frame.frame.width() as usize,
                                frame.frame.height() as usize,
                            );
                        }
                        Err(error) => {
                            self.stats.import_failures += 1;
                            eprintln!("Kuroko presenter video import failed: {error}");
                        }
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            }
        }
    }

    fn update_overlay(&mut self, pts: Duration, width: usize, height: usize) {
        let mut overlay = self.overlay.render(
            pts,
            OverlayViewport::new(
                width.min(u32::MAX as usize) as u32,
                height.min(u32::MAX as usize) as u32,
            ),
        );
        self.subtitles.append_to_overlay(pts, &mut overlay);
        if !overlay.is_empty() {
            self.stats.overlay_frames += 1;
        }
        self.current_overlay = Some(overlay);
    }

    fn pump_subtitles(&mut self) {
        loop {
            match self.subtitle_frames.try_recv() {
                Ok(frame) => {
                    self.stats.decoded_subtitle_frames += 1;
                    self.subtitles.push(frame);
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            }
        }
    }

    fn pump_audio(&mut self) {
        loop {
            match self.audio_frames.try_recv() {
                Ok(frame) => {
                    self.push_audio(frame);
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            }
        }
        if self.audio_started {
            self.sync_player_to_audio_output();
        }
    }

    fn sync_player_to_audio_output(&self) {
        let Ok(snapshot) = self.audio_output.clock_snapshot() else {
            return;
        };
        let _ = self.player.update_audio_clock(snapshot);
    }

    fn push_audio(&mut self, frame: PlayerAudioFrame) {
        if !self.audio_started {
            if let Err(error) = self.audio_output.configure(frame.frame.format) {
                self.stats.audio_failures += 1;
                eprintln!("Kuroko presenter CoreAudio configure failed: {error}");
                return;
            }
            if let Err(error) = self.audio_output.start() {
                self.stats.audio_failures += 1;
                eprintln!("Kuroko presenter CoreAudio start failed: {error}");
                return;
            }
            self.audio_started = true;
        }
        match self.audio_output.push(frame.frame) {
            Ok(_) => self.stats.pushed_audio_frames += 1,
            Err(error) => {
                self.stats.audio_failures += 1;
                eprintln!("Kuroko presenter CoreAudio push failed: {error}");
            }
        }
    }
}

impl Drop for PresenterRuntime {
    fn drop(&mut self) {
        let _ = self.audio_output.stop();
        let _ = self.player.close();
    }
}

fn build_renderer(
    preference: RendererBackendPreference,
    metal_config: MetalRendererConfig,
) -> Result<Box<dyn RendererBackend>> {
    match preference {
        RendererBackendPreference::PlatformNative | RendererBackendPreference::Auto => {
            Ok(Box::new(MetalRenderer::with_config(metal_config)?))
        }
        RendererBackendPreference::WgpuFallback => build_wgpu_renderer(),
        RendererBackendPreference::FlutterTexture => Err(PlayerError::Renderer(
            "Flutter texture backend is not supported by the presenter runtime".to_string(),
        )),
    }
}

#[cfg(feature = "wgpu")]
fn build_wgpu_renderer() -> Result<Box<dyn RendererBackend>> {
    Ok(Box::new(crate::renderer::wgpu::WgpuRenderer::new()?))
}

#[cfg(not(feature = "wgpu"))]
fn build_wgpu_renderer() -> Result<Box<dyn RendererBackend>> {
    Err(PlayerError::Renderer(
        "wgpu renderer backend requires the `wgpu` cargo feature".to_string(),
    ))
}

fn subtitle_is_active(frame: &PlayerSubtitleFrame, pts: Duration) -> bool {
    if frame.frame.is_empty() {
        return false;
    }
    if subtitle_start(frame).is_some_and(|start| pts < start) {
        return false;
    }
    if frame.frame.end.is_some_and(|end| pts >= end) {
        return false;
    }
    true
}

fn subtitle_start(frame: &PlayerSubtitleFrame) -> Option<Duration> {
    frame.frame.start.or(frame.pts)
}

#[derive(Debug, Default)]
struct SubtitleFrameState {
    frames: Vec<PlayerSubtitleFrame>,
    #[cfg(feature = "libass")]
    text_renderer: CachedLibassTextRenderer,
}

impl SubtitleFrameState {
    fn push(&mut self, frame: PlayerSubtitleFrame) {
        self.retain_at(subtitle_start(&frame).unwrap_or(frame.media_time));
        if frame.frame.is_empty() {
            self.frames
                .retain(|current| current.frame.track_id != frame.frame.track_id);
            return;
        }
        if frame.frame.end.is_none() {
            self.frames
                .retain(|current| current.frame.track_id != frame.frame.track_id);
        }
        self.frames.push(frame);
        self.frames
            .sort_by_key(|frame| subtitle_start(frame).unwrap_or(frame.media_time));
    }

    fn retain_at(&mut self, pts: Duration) {
        self.frames
            .retain(|frame| !frame.frame.is_empty() && frame.frame.end.is_none_or(|end| pts < end));
    }

    fn append_to_overlay(&mut self, pts: Duration, overlay: &mut OverlayFrame) {
        self.retain_at(pts);
        let active = self
            .frames
            .iter()
            .filter(|frame| subtitle_is_active(frame, pts))
            .collect::<Vec<_>>();
        if active.is_empty() {
            return;
        }

        let mut subtitle_changed = false;
        for frame in &active {
            if !frame.frame.bitmap.planes.is_empty() {
                overlay
                    .subtitle_planes
                    .extend(frame.frame.bitmap.planes.iter().cloned());
                subtitle_changed = true;
            }
        }

        let text_frames = active
            .iter()
            .filter(|frame| frame.frame.has_text())
            .map(|frame| frame.frame.clone())
            .collect::<Vec<_>>();
        if !text_frames.is_empty() {
            self.append_text_subtitles(pts, overlay, &text_frames);
            subtitle_changed = true;
        }

        overlay.subtitle_changed |= subtitle_changed;
    }

    #[cfg(feature = "libass")]
    fn append_text_subtitles(
        &mut self,
        pts: Duration,
        overlay: &mut OverlayFrame,
        frames: &[DecodedSubtitleFrame],
    ) {
        match self.text_renderer.render(pts, overlay.viewport, frames) {
            Ok(Some(frame)) => overlay.subtitle_planes.extend(frame.planes),
            Ok(None) => {}
            Err(error) => {
                eprintln!("Kuroko presenter text subtitle render failed: {error}");
                append_text_subtitles_debug(pts, overlay, frames);
            }
        }
    }

    #[cfg(not(feature = "libass"))]
    fn append_text_subtitles(
        &mut self,
        pts: Duration,
        overlay: &mut OverlayFrame,
        frames: &[DecodedSubtitleFrame],
    ) {
        append_text_subtitles_debug(pts, overlay, frames);
    }
}

#[cfg(feature = "libass")]
#[derive(Debug, Default)]
struct CachedLibassTextRenderer {
    script: Option<String>,
    renderer: Option<LibassSubtitleRenderer>,
}

#[cfg(feature = "libass")]
impl CachedLibassTextRenderer {
    fn render(
        &mut self,
        pts: Duration,
        viewport: OverlayViewport,
        frames: &[DecodedSubtitleFrame],
    ) -> crate::subtitle::Result<Option<crate::subtitle::SubtitleFrame>> {
        let fallback_end = pts.saturating_add(Duration::from_secs(24 * 60 * 60));
        let Some(script) = decoded_subtitle_frames_to_ass_script(frames.iter(), fallback_end)
        else {
            self.script = None;
            self.renderer = None;
            return Ok(None);
        };
        if self.script.as_ref() != Some(&script) {
            self.renderer = Some(LibassSubtitleRenderer::from_ass_script(
                script.as_bytes(),
                LibassRenderConfig::default(),
            )?);
            self.script = Some(script);
        }

        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(None);
        };
        renderer
            .render(SubtitleRenderRequest::new(
                pts,
                viewport.width,
                viewport.height,
            ))
            .map(|output| Some(output.into_rgba_frame()))
    }
}

fn append_text_subtitles_debug(
    pts: Duration,
    overlay: &mut OverlayFrame,
    frames: &[DecodedSubtitleFrame],
) {
    let fallback_end = pts.saturating_add(Duration::from_secs(24 * 60 * 60));
    let timeline = decoded_subtitle_frames_to_timeline(frames.iter(), fallback_end);
    let frame = SubtitleRendererCore::new_debug(timeline)
        .render(
            pts,
            SubtitleViewport::new(overlay.viewport.width, overlay.viewport.height),
        )
        .frame;
    overlay.subtitle_planes.extend(frame.planes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtitle::{
        DecodedSubtitleFrame, SubtitleBitmapPlane, SubtitleTextFormat, SubtitleTextSegment,
    };

    fn subtitle_frame(start: Duration, end: Option<Duration>) -> PlayerSubtitleFrame {
        let mut frame = DecodedSubtitleFrame::new(2, Some(start), end);
        frame.push_bitmap_plane(
            SubtitleBitmapPlane {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
            },
            false,
        );
        PlayerSubtitleFrame {
            frame,
            pts: Some(start),
            media_time: start,
            late_by: None,
        }
    }

    fn text_subtitle_frame(
        track_id: i64,
        start: Duration,
        end: Option<Duration>,
        text: &str,
    ) -> PlayerSubtitleFrame {
        let mut frame = DecodedSubtitleFrame::new(track_id, Some(start), end);
        frame.push_text(SubtitleTextSegment::new(
            SubtitleTextFormat::PlainText,
            text,
        ));
        PlayerSubtitleFrame {
            frame,
            pts: Some(start),
            media_time: start,
            late_by: None,
        }
    }

    fn empty_overlay() -> OverlayFrame {
        OverlayFrame {
            pts: Duration::ZERO,
            viewport: OverlayViewport::new(640, 360),
            subtitle_planes: Vec::new(),
            subtitle_alpha_planes: Vec::new(),
            subtitle_changed: false,
            danmaku_boxes: Vec::new(),
        }
    }

    #[test]
    fn subtitle_active_window_respects_start_end_and_empty_frames() {
        let active = subtitle_frame(Duration::from_secs(1), Some(Duration::from_secs(3)));

        assert!(!subtitle_is_active(&active, Duration::from_millis(999)));
        assert!(subtitle_is_active(&active, Duration::from_secs(1)));
        assert!(subtitle_is_active(&active, Duration::from_millis(2999)));
        assert!(!subtitle_is_active(&active, Duration::from_secs(3)));

        let empty = PlayerSubtitleFrame {
            frame: DecodedSubtitleFrame::new(2, Some(Duration::ZERO), None),
            pts: Some(Duration::ZERO),
            media_time: Duration::ZERO,
            late_by: None,
        };
        assert!(!subtitle_is_active(&empty, Duration::ZERO));
    }

    #[test]
    fn subtitle_state_keeps_overlapping_bitmap_frames() {
        let mut state = SubtitleFrameState::default();
        state.push(subtitle_frame(
            Duration::from_secs(1),
            Some(Duration::from_secs(4)),
        ));
        state.push(subtitle_frame(
            Duration::from_secs(2),
            Some(Duration::from_secs(5)),
        ));
        let mut overlay = empty_overlay();

        state.append_to_overlay(Duration::from_secs(3), &mut overlay);

        assert_eq!(overlay.subtitle_planes.len(), 2);
        assert!(overlay.subtitle_changed);
    }

    #[test]
    fn subtitle_state_expires_old_frames_and_empty_frame_clears_track() {
        let mut state = SubtitleFrameState::default();
        state.push(subtitle_frame(
            Duration::from_secs(1),
            Some(Duration::from_secs(2)),
        ));
        state.push(subtitle_frame(
            Duration::from_secs(3),
            Some(Duration::from_secs(5)),
        ));
        let mut overlay = empty_overlay();

        state.append_to_overlay(Duration::from_secs(4), &mut overlay);

        assert_eq!(overlay.subtitle_planes.len(), 1);

        state.push(PlayerSubtitleFrame {
            frame: DecodedSubtitleFrame::new(2, Some(Duration::from_secs(4)), None),
            pts: Some(Duration::from_secs(4)),
            media_time: Duration::from_secs(4),
            late_by: None,
        });
        let mut overlay = empty_overlay();
        state.append_to_overlay(Duration::from_millis(4500), &mut overlay);

        assert!(overlay.subtitle_planes.is_empty());
    }

    #[test]
    fn subtitle_state_renders_text_frames_into_overlay() {
        let mut state = SubtitleFrameState::default();
        state.push(text_subtitle_frame(
            7,
            Duration::from_secs(1),
            Some(Duration::from_secs(3)),
            "hello",
        ));
        let mut overlay = empty_overlay();

        state.append_to_overlay(Duration::from_secs(2), &mut overlay);

        assert!(!overlay.subtitle_planes.is_empty());
        assert!(overlay.subtitle_changed);
    }
}
