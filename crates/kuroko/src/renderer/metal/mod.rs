use std::ffi::c_void;

use crate::core::{PlatformSurface, RendererBackend, Result};
use crate::overlay::OverlayFrame;

#[cfg(target_os = "macos")]
mod macos;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClearColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

impl ClearColor {
    pub fn animated(time_seconds: f64) -> Self {
        Self {
            red: time_seconds.sin() * 0.5 + 0.5,
            green: (time_seconds * 0.73).sin() * 0.5 + 0.5,
            blue: (time_seconds * 1.37).cos() * 0.5 + 0.5,
            alpha: 1.0,
        }
    }
}

pub struct MetalRenderer {
    #[cfg(target_os = "macos")]
    inner: macos::MetalRendererImpl,
    #[cfg(not(target_os = "macos"))]
    _unsupported: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MetalRendererStats {
    pub drawable_width: u32,
    pub drawable_height: u32,
    pub rendered_frames: u64,
    pub prepared_overlay_frames: u64,
    pub prepared_overlay_subtitle_planes: u64,
    pub prepared_overlay_danmaku_boxes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFrameTextureSource {
    pub raw_pixel_buffer: *mut c_void,
    pub width: u32,
    pub height: u32,
}

impl VideoFrameTextureSource {
    pub fn new(raw_pixel_buffer: *mut c_void, width: u32, height: u32) -> Self {
        Self {
            raw_pixel_buffer,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportedVideoFormat {
    Nv12,
    P010,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedVideoFrameInfo {
    pub width: usize,
    pub height: usize,
    pub pixel_format: u32,
    pub pixel_format_fourcc: String,
    pub format: ImportedVideoFormat,
    pub full_range: bool,
    pub planes: Vec<ImportedVideoPlaneInfo>,
}

pub struct ImportedVideoFrame {
    info: ImportedVideoFrameInfo,
    #[cfg(target_os = "macos")]
    inner: macos::ImportedVideoFrameTextures,
    #[cfg(not(target_os = "macos"))]
    _unsupported: (),
}

impl ImportedVideoFrame {
    pub fn info(&self) -> &ImportedVideoFrameInfo {
        &self.info
    }

    pub fn plane_count(&self) -> usize {
        #[cfg(target_os = "macos")]
        {
            self.inner.plane_count()
        }
        #[cfg(not(target_os = "macos"))]
        {
            0
        }
    }
}

pub struct VideoRenderFrame<'a> {
    pub frame: &'a ImportedVideoFrame,
    pub full_range: bool,
}

impl<'a> VideoRenderFrame<'a> {
    pub fn new(frame: &'a ImportedVideoFrame) -> Self {
        Self {
            frame,
            full_range: frame.info.full_range,
        }
    }

    pub fn full_range(mut self, full_range: bool) -> Self {
        self.full_range = full_range;
        self
    }
}

pub struct OverlayRenderFrame<'a> {
    pub frame: &'a OverlayFrame,
}

impl<'a> OverlayRenderFrame<'a> {
    pub fn new(frame: &'a OverlayFrame) -> Self {
        Self { frame }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedOverlayFrameInfo {
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub subtitle_planes: usize,
    pub subtitle_pixels: usize,
    pub subtitle_bytes: usize,
    pub danmaku_boxes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedVideoPlaneInfo {
    pub index: usize,
    pub width: usize,
    pub height: usize,
    pub metal_pixel_format: &'static str,
}

impl MetalRenderer {
    pub fn new() -> Result<Self> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                inner: macos::MetalRendererImpl::new()?,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(PlayerError::Renderer(
                "Metal renderer is only available on macOS for v0".to_string(),
            ))
        }
    }

    pub unsafe fn attach_raw_layer(
        &mut self,
        layer: *mut c_void,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            unsafe { self.inner.attach_raw_layer(layer, width, height, scale) }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (layer, width, height, scale);
            Err(PlayerError::Renderer(
                "Metal renderer is only available on macOS for v0".to_string(),
            ))
        }
    }

    pub unsafe fn import_video_frame_textures(
        &mut self,
        source: VideoFrameTextureSource,
    ) -> Result<ImportedVideoFrame> {
        #[cfg(target_os = "macos")]
        {
            let imported = unsafe { self.inner.import_video_frame_textures(source) }?;
            Ok(ImportedVideoFrame {
                info: imported.info,
                inner: imported.textures,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = source;
            Err(PlayerError::Renderer(
                "Metal renderer is only available on macOS for v0".to_string(),
            ))
        }
    }

    pub fn render_video_frame(&mut self, frame: VideoRenderFrame<'_>) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.inner.render_video_frame(frame)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = frame;
            Err(PlayerError::Renderer(
                "Metal renderer is only available on macOS for v0".to_string(),
            ))
        }
    }

    pub fn render_video_frame_with_overlay(
        &mut self,
        frame: VideoRenderFrame<'_>,
        overlay: OverlayRenderFrame<'_>,
    ) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.inner.render_video_frame_with_overlay(frame, overlay)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (frame, overlay);
            Err(PlayerError::Renderer(
                "Metal renderer is only available on macOS for v0".to_string(),
            ))
        }
    }

    pub fn render_overlay_frame(&mut self, overlay: OverlayRenderFrame<'_>) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.inner.render_overlay_frame(overlay)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = overlay;
            Err(PlayerError::Renderer(
                "Metal renderer is only available on macOS for v0".to_string(),
            ))
        }
    }

    pub fn prepare_overlay_frame(
        &mut self,
        frame: OverlayRenderFrame<'_>,
    ) -> Result<PreparedOverlayFrameInfo> {
        let info = inspect_overlay_frame(frame.frame)?;
        #[cfg(target_os = "macos")]
        {
            self.inner.record_prepared_overlay_frame(info);
        }
        Ok(info)
    }

    pub fn stats(&self) -> MetalRendererStats {
        #[cfg(target_os = "macos")]
        {
            self.inner.stats()
        }
        #[cfg(not(target_os = "macos"))]
        {
            MetalRendererStats::default()
        }
    }
}

fn inspect_overlay_frame(frame: &OverlayFrame) -> Result<PreparedOverlayFrameInfo> {
    let mut subtitle_pixels = 0usize;
    let mut subtitle_bytes = 0usize;
    for plane in &frame.subtitle_planes {
        let pixels = plane.width as usize * plane.height as usize;
        let bytes = pixels * 4;
        if plane.rgba.len() != bytes {
            return Err(crate::core::PlayerError::Renderer(format!(
                "subtitle plane has {} bytes, expected {bytes} for {}x{} RGBA",
                plane.rgba.len(),
                plane.width,
                plane.height
            )));
        }
        subtitle_pixels += pixels;
        subtitle_bytes += bytes;
    }

    Ok(PreparedOverlayFrameInfo {
        viewport_width: frame.viewport.width,
        viewport_height: frame.viewport.height,
        subtitle_planes: frame.subtitle_planes.len(),
        subtitle_pixels,
        subtitle_bytes,
        danmaku_boxes: frame.danmaku_boxes.len(),
    })
}

pub fn fourcc_string(value: u32) -> String {
    let bytes = value.to_be_bytes();
    if bytes
        .iter()
        .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
    {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        format!("0x{value:08x}")
    }
}

impl RendererBackend for MetalRenderer {
    fn attach_surface(&mut self, surface: PlatformSurface) -> Result<()> {
        match surface {
            PlatformSurface::Metal(handle) => unsafe {
                self.attach_raw_layer(
                    handle.raw_layer as *mut c_void,
                    handle.width,
                    handle.height,
                    handle.scale,
                )
            },
            PlatformSurface::Wgpu(_) => Err(crate::core::PlayerError::Renderer(
                "wgpu surface cannot be attached to MetalRenderer".to_string(),
            )),
            PlatformSurface::FlutterTexture(_) => Err(crate::core::PlayerError::Renderer(
                "Flutter texture cannot be attached to MetalRenderer".to_string(),
            )),
        }
    }

    fn detach_surface(&mut self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.inner.detach_surface();
        }
        Ok(())
    }

    fn resize_surface(&mut self, width: u32, height: u32, scale: f64) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.inner.resize_surface(width, height, scale);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (width, height, scale);
        }
        Ok(())
    }

    fn render_test_frame(&mut self, time_seconds: f64) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.inner.render_clear(ClearColor::animated(time_seconds))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = time_seconds;
            Err(PlayerError::Renderer(
                "Metal renderer is only available on macOS for v0".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::danmaku::DanmakuLayoutBox;
    use crate::overlay::OverlayViewport;
    use crate::subtitle::SubtitleBitmapPlane;

    #[test]
    fn inspect_overlay_counts_subtitle_bytes_and_danmaku_boxes() {
        let frame = OverlayFrame {
            pts: Duration::from_secs(1),
            viewport: OverlayViewport::new(640, 360),
            subtitle_planes: vec![SubtitleBitmapPlane {
                x: 0,
                y: 0,
                width: 10,
                height: 4,
                rgba: vec![255; 10 * 4 * 4],
            }],
            danmaku_boxes: vec![DanmakuLayoutBox {
                item_id: 1,
                x: 12.0,
                y: 24.0,
                width: 80.0,
                height: 24.0,
            }],
        };

        let info = inspect_overlay_frame(&frame).unwrap();

        assert_eq!(info.viewport_width, 640);
        assert_eq!(info.viewport_height, 360);
        assert_eq!(info.subtitle_planes, 1);
        assert_eq!(info.subtitle_pixels, 40);
        assert_eq!(info.subtitle_bytes, 160);
        assert_eq!(info.danmaku_boxes, 1);
    }

    #[test]
    fn inspect_overlay_rejects_malformed_rgba_plane() {
        let frame = OverlayFrame {
            pts: Duration::ZERO,
            viewport: OverlayViewport::new(1, 1),
            subtitle_planes: vec![SubtitleBitmapPlane {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                rgba: vec![0; 15],
            }],
            danmaku_boxes: Vec::new(),
        };

        assert!(inspect_overlay_frame(&frame).is_err());
    }
}
