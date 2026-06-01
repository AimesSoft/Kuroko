use std::ffi::c_void;

use crate::core::{ColorPrimaries, PlatformSurface, RendererBackend, Result, TransferFunction};
use crate::overlay::OverlayFrame;
use crate::renderer::pipeline::{
    ColorRange, HdrMetadata, MatrixCoefficients, SourceColorState, VideoRenderPipeline,
};

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetalRendererConfig {
    pub output_mode: MetalOutputMode,
}

impl Default for MetalRendererConfig {
    fn default() -> Self {
        Self {
            output_mode: MetalOutputMode::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetalOutputMode {
    Sdr,
    AppleEdr { headroom: f32 },
}

impl MetalOutputMode {
    pub fn apple_edr(headroom: f32) -> Self {
        Self::AppleEdr {
            headroom: headroom.max(1.0),
        }
    }

    pub fn pixel_format(self) -> MetalDrawablePixelFormat {
        match self {
            Self::Sdr => MetalDrawablePixelFormat::Bgra8Unorm,
            Self::AppleEdr { .. } => MetalDrawablePixelFormat::Rgba16Float,
        }
    }

    pub fn is_edr(self) -> bool {
        matches!(self, Self::AppleEdr { .. })
    }

    pub fn target_color(self) -> crate::renderer::pipeline::TargetColorState {
        match self {
            Self::Sdr => crate::renderer::pipeline::TargetColorState::sdr(ColorPrimaries::Bt709),
            Self::AppleEdr { headroom } => crate::renderer::pipeline::TargetColorState::apple_edr(
                ColorPrimaries::Bt709,
                headroom,
            ),
        }
    }
}

impl Default for MetalOutputMode {
    fn default() -> Self {
        Self::Sdr
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetalDrawablePixelFormat {
    Bgra8Unorm,
    Rgba16Float,
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
    pub color_range: ColorRange,
    pub planes: Vec<ImportedVideoPlaneInfo>,
}

pub struct ImportedVideoFrame {
    info: ImportedVideoFrameInfo,
    source_color: SourceColorState,
    #[cfg(target_os = "macos")]
    inner: Option<macos::ImportedVideoFrameTextures>,
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
            self.inner.as_ref().map_or(0, |inner| inner.plane_count())
        }
        #[cfg(not(target_os = "macos"))]
        {
            0
        }
    }

    pub fn source_color(&self) -> SourceColorState {
        self.source_color
    }

    pub fn set_source_color(&mut self, source: SourceColorState) {
        let import_range = self.info.color_range;
        let fallback = source.range.resolve(ColorRange::Limited);
        self.source_color = source.range(import_range.resolve(fallback));
    }

    pub fn set_source_color_metadata(
        &mut self,
        primaries: ColorPrimaries,
        transfer: TransferFunction,
        range: ColorRange,
        matrix: MatrixCoefficients,
        hdr_metadata: Option<HdrMetadata>,
    ) {
        self.set_source_color(
            SourceColorState::new(primaries, transfer)
                .range(range)
                .matrix(matrix)
                .hdr_metadata(hdr_metadata),
        );
    }
}

pub struct VideoRenderFrame<'a> {
    pub frame: &'a ImportedVideoFrame,
    pub pipeline: VideoRenderPipeline,
}

impl<'a> VideoRenderFrame<'a> {
    pub fn new(frame: &'a ImportedVideoFrame) -> Self {
        Self {
            frame,
            pipeline: VideoRenderPipeline::new(frame.source_color(), Default::default()),
        }
    }

    pub fn full_range(mut self, full_range: bool) -> Self {
        self.pipeline.source.range = color_range_from_import(full_range);
        self
    }

    pub fn source_color(mut self, primaries: ColorPrimaries, transfer: TransferFunction) -> Self {
        let range = self.pipeline.source.range;
        let matrix = self.pipeline.source.matrix;
        let target = self.pipeline.target;
        self.pipeline = VideoRenderPipeline::new(
            SourceColorState::new(primaries, transfer)
                .range(range)
                .matrix(matrix),
            target,
        );
        self
    }

    pub fn pipeline(mut self, pipeline: VideoRenderPipeline) -> Self {
        self.pipeline = pipeline;
        self
    }
}

fn color_range_from_import(full_range: bool) -> ColorRange {
    if full_range {
        ColorRange::Full
    } else {
        ColorRange::Limited
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
        Self::with_config(MetalRendererConfig::default())
    }

    pub fn with_config(config: MetalRendererConfig) -> Result<Self> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                inner: macos::MetalRendererImpl::new(config)?,
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
            let source_color =
                SourceColorState::new(ColorPrimaries::Unknown, TransferFunction::Unknown)
                    .range(imported.info.color_range.resolve(ColorRange::Limited));
            Ok(ImportedVideoFrame {
                info: imported.info,
                source_color,
                inner: Some(imported.textures),
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
    use crate::renderer::pipeline::{MatrixCoefficients, SourceColorState};
    use crate::subtitle::SubtitleBitmapPlane;

    fn test_imported_frame(
        import_range: ColorRange,
        source_color: SourceColorState,
    ) -> ImportedVideoFrame {
        ImportedVideoFrame {
            info: ImportedVideoFrameInfo {
                width: 1920,
                height: 1080,
                pixel_format: 0,
                pixel_format_fourcc: "test".to_string(),
                format: ImportedVideoFormat::P010,
                full_range: matches!(import_range, ColorRange::Full),
                color_range: import_range,
                planes: Vec::new(),
            },
            source_color,
            #[cfg(target_os = "macos")]
            inner: None,
            #[cfg(not(target_os = "macos"))]
            _unsupported: (),
        }
    }

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

    #[test]
    fn metal_output_mode_maps_sdr_to_default_drawable_and_target() {
        let output = MetalOutputMode::default();

        assert_eq!(output.pixel_format(), MetalDrawablePixelFormat::Bgra8Unorm);
        assert!(!output.is_edr());

        let target = output.target_color();
        assert_eq!(target.primaries, ColorPrimaries::Bt709);
        assert_eq!(target.transfer, TransferFunction::Srgb);
        assert_eq!(target.peak_nits, 100.0);
        assert_eq!(target.edr_headroom, 1.0);
    }

    #[test]
    fn metal_output_mode_maps_apple_edr_to_float_drawable_and_headroom_target() {
        let output = MetalOutputMode::apple_edr(4.0);

        assert_eq!(output.pixel_format(), MetalDrawablePixelFormat::Rgba16Float);
        assert!(output.is_edr());

        let target = output.target_color();
        assert_eq!(target.primaries, ColorPrimaries::Bt709);
        assert_eq!(target.transfer, TransferFunction::Srgb);
        assert_eq!(target.peak_nits, 812.0);
        assert_eq!(target.reference_white_nits, 203.0);
        assert_eq!(target.edr_headroom, 4.0);
    }

    #[test]
    fn metal_output_mode_clamps_edr_headroom_to_one() {
        let target = MetalOutputMode::apple_edr(0.25).target_color();

        assert_eq!(target.peak_nits, 203.0);
        assert_eq!(target.edr_headroom, 1.0);
    }

    #[test]
    fn video_render_frame_uses_imported_source_color() {
        let source = SourceColorState::new(ColorPrimaries::Bt2020, TransferFunction::Pq)
            .range(ColorRange::Limited)
            .matrix(MatrixCoefficients::Bt2020NonConstantLuminance);
        let frame = test_imported_frame(ColorRange::Limited, source);

        let render_frame = VideoRenderFrame::new(&frame);

        assert_eq!(
            render_frame.pipeline.source.primaries,
            ColorPrimaries::Bt2020
        );
        assert_eq!(render_frame.pipeline.source.transfer, TransferFunction::Pq);
        assert_eq!(render_frame.pipeline.source.range, ColorRange::Limited);
        assert_eq!(
            render_frame.pipeline.source.matrix,
            MatrixCoefficients::Bt2020NonConstantLuminance
        );
    }

    #[test]
    fn imported_frame_prefers_pixel_buffer_range_over_metadata() {
        let mut frame = test_imported_frame(
            ColorRange::Full,
            SourceColorState::new(ColorPrimaries::Unknown, TransferFunction::Unknown)
                .range(ColorRange::Full),
        );

        frame.set_source_color_metadata(
            ColorPrimaries::Bt709,
            TransferFunction::Srgb,
            ColorRange::Limited,
            MatrixCoefficients::Bt709,
            None,
        );

        assert_eq!(frame.source_color().range, ColorRange::Full);
        assert_eq!(frame.source_color().matrix, MatrixCoefficients::Bt709);
    }

    #[test]
    fn imported_frame_applies_hdr_metadata_peak() {
        let mut frame = test_imported_frame(
            ColorRange::Limited,
            SourceColorState::new(ColorPrimaries::Unknown, TransferFunction::Unknown),
        );
        let metadata = HdrMetadata::new(
            None,
            Some(crate::renderer::pipeline::ContentLightMetadata {
                max_content_light_level_nits: 4000,
                max_frame_average_light_level_nits: 450,
            }),
        );

        frame.set_source_color_metadata(
            ColorPrimaries::Bt2020,
            TransferFunction::Pq,
            ColorRange::Limited,
            MatrixCoefficients::Bt2020NonConstantLuminance,
            Some(metadata),
        );

        assert_eq!(frame.source_color().hdr_metadata, Some(metadata));
        assert_eq!(frame.source_color().nominal_peak_nits, 4000.0);
    }

    #[test]
    fn imported_frame_uses_metadata_range_when_import_unspecified() {
        let mut frame = test_imported_frame(
            ColorRange::Unspecified,
            SourceColorState::new(ColorPrimaries::Unknown, TransferFunction::Unknown)
                .range(ColorRange::Unspecified),
        );

        frame.set_source_color(
            SourceColorState::new(ColorPrimaries::Bt709, TransferFunction::Srgb)
                .range(ColorRange::Full),
        );

        assert_eq!(frame.source_color().range, ColorRange::Full);
    }
}
