use std::time::Duration;

use crossbeam_channel::Receiver;

use crate::apple::coreaudio::{CoreAudioOutput, CoreAudioOutputConfig};
use crate::core::{
    MediaRequest, PlatformSurface, Player, PlayerAudioFrame, PlayerConfig, PlayerSubtitleFrame,
    PlayerVideoFrame, RendererBackend,
};
use crate::ffmpeg::Frame;
use crate::overlay::{OverlayFrame, OverlayTimeline, OverlayViewport};
use crate::renderer::metal::{
    ImportedVideoFrame, MetalRenderer, MetalRendererConfig, OverlayRenderFrame,
    VideoFrameTextureSource, VideoRenderFrame,
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
    renderer: MetalRenderer,
    video_frames: Receiver<PlayerVideoFrame>,
    audio_frames: Receiver<PlayerAudioFrame>,
    subtitle_frames: Receiver<PlayerSubtitleFrame>,
    audio_output: CoreAudioOutput,
    audio_started: bool,
    current_frame: Option<ImportedVideoFrame>,
    current_overlay: Option<OverlayFrame>,
    current_subtitle: Option<PlayerSubtitleFrame>,
    overlay: OverlayTimeline,
    stats: PresenterStats,
}

impl PresenterRuntime {
    pub fn new(config: PresenterConfig) -> Result<Self> {
        let player = Player::new(config.player);
        let video_frames = player.subscribe_video_frames();
        let audio_frames = player.subscribe_audio_frames();
        let subtitle_frames = player.subscribe_subtitle_frames();
        Ok(Self {
            player,
            renderer: MetalRenderer::with_config(config.renderer)?,
            video_frames,
            audio_frames,
            subtitle_frames,
            audio_output: CoreAudioOutput::new(config.audio),
            audio_started: false,
            current_frame: None,
            current_overlay: None,
            current_subtitle: None,
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

    pub fn render_tick(&mut self, time_seconds: f64) -> Result<PresenterStats> {
        self.pump_audio();
        self.pump_subtitles();
        self.pump_video();

        if let Some(frame) = &self.current_frame {
            let result = match &self.current_overlay {
                Some(overlay) => self.renderer.render_video_frame_with_overlay(
                    VideoRenderFrame::new(frame),
                    OverlayRenderFrame::new(overlay),
                ),
                None => self
                    .renderer
                    .render_video_frame(VideoRenderFrame::new(frame)),
            };
            match result {
                Ok(()) => self.stats.rendered_video_frames += 1,
                Err(error) => {
                    self.stats.render_failures += 1;
                    return Err(error);
                }
            }
        } else {
            self.renderer.render_test_frame(time_seconds)?;
            self.stats.rendered_test_frames += 1;
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
                    match import_video_frame(&mut self.renderer, &frame.frame) {
                        Ok(imported) => {
                            let info = imported.info();
                            let pts = frame.pts.unwrap_or(frame.media_time);
                            self.update_overlay(pts, info.width, info.height);
                            self.current_frame = Some(imported);
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
        if let Some(subtitle) = self.current_subtitle.as_ref() {
            if subtitle_is_active(subtitle, pts) {
                overlay
                    .subtitle_planes
                    .extend(subtitle.frame.bitmap.planes.iter().cloned());
                overlay.subtitle_changed = true;
            }
        }
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
                    self.current_subtitle = Some(frame);
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

fn import_video_frame(renderer: &mut MetalRenderer, frame: &Frame) -> Result<ImportedVideoFrame> {
    let pixel_buffer = frame.videotoolbox_pixel_buffer().ok_or_else(|| {
        PlayerError::Renderer(
            "decoded frame is not backed by VideoToolbox CVPixelBuffer".to_string(),
        )
    })?;
    let mut imported = unsafe {
        renderer.import_video_frame_textures(VideoFrameTextureSource::new(
            pixel_buffer.raw(),
            pixel_buffer.width(),
            pixel_buffer.height(),
        ))
    }?;
    imported.set_source_color_metadata(
        frame.color_primaries(),
        frame.transfer_function(),
        frame.color_range(),
        frame.matrix_coefficients(),
        frame.hdr_metadata(),
    );
    Ok(imported)
}

fn subtitle_is_active(frame: &PlayerSubtitleFrame, pts: Duration) -> bool {
    if frame.frame.is_empty() {
        return false;
    }
    if frame.pts.is_some_and(|start| pts < start) {
        return false;
    }
    if frame.frame.end.is_some_and(|end| pts >= end) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtitle::{DecodedSubtitleFrame, SubtitleBitmapPlane};

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
}
