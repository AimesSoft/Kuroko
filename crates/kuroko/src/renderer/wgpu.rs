use std::ffi::c_void;

use wgpu::util::DeviceExt;

use crate::core::{
    ColorPrimaries, PlatformSurface, PlayerError, PlayerVideoFrame, RendererBackend, Result,
    TransferFunction, WgpuSurfaceHandle, WgpuSurfaceKind,
};
use crate::overlay::OverlayFrame;
use crate::renderer::pipeline::{
    ColorRange, SourceColorState, TargetColorState, ToneMapOperator, VideoRenderPipeline,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WgpuRendererStats {
    pub surface_width: u32,
    pub surface_height: u32,
    pub rendered_frames: u64,
    pub offscreen_frames: u64,
    pub attached: bool,
}

/// A clear color in the renderer's working space, components in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WgpuClearColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

impl WgpuClearColor {
    pub fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// An animated test pattern, matching the Metal renderer's `ClearColor::animated`
    /// so the two backends can be compared frame-for-frame.
    pub fn animated(time_seconds: f64) -> Self {
        Self {
            red: time_seconds.sin() * 0.5 + 0.5,
            green: (time_seconds * 0.73).sin() * 0.5 + 0.5,
            blue: (time_seconds * 1.37).cos() * 0.5 + 0.5,
            alpha: 1.0,
        }
    }

    fn to_wgpu(self) -> wgpu::Color {
        wgpu::Color {
            r: self.red,
            g: self.green,
            b: self.blue,
            a: self.alpha,
        }
    }
}

/// Tightly packed RGBA8 pixels read back from an offscreen render target.
///
/// Used as the headless verification oracle for the wgpu backend: render a pass,
/// copy the target to host memory, and assert the pixels are what we expect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WgpuOffscreenReadback {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl WgpuOffscreenReadback {
    /// Returns the RGBA bytes of the pixel at `(x, y)`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[offset],
            self.rgba[offset + 1],
            self.rgba[offset + 2],
            self.rgba[offset + 3],
        ]
    }
}

/// Fragment-shader uniforms for the video pipeline. The field order and byte layout
/// mirror the Metal `VideoUniforms` in `renderer/metal/macos.rs` exactly, so both
/// backends consume the same data and produce the same pixels.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VideoUniforms {
    pub is_p010: u32,
    pub full_range: u32,
    pub source_transfer: u32,
    pub target_transfer: u32,
    pub tone_map: u32,
    pub edr_output: u32,
    pub reserved0: u32,
    pub reserved1: u32,
    pub nits: [f32; 4],
    pub luma_coefficients: [f32; 4],
    pub gamut_matrix_rows: [[f32; 4]; 3],
}

impl VideoUniforms {
    /// Build the uniform block from a resolved render pipeline, matching how the
    /// Metal renderer fills its `VideoUniforms` in `render_video_frame`.
    pub fn from_pipeline(pipeline: &VideoRenderPipeline, is_p010: bool, edr_output: bool) -> Self {
        let luma = pipeline.luma_coefficients();
        Self {
            is_p010: u32::from(is_p010),
            full_range: u32::from(matches!(pipeline.source.range, ColorRange::Full)),
            source_transfer: transfer_code(pipeline.source.transfer),
            target_transfer: transfer_code(pipeline.target.transfer),
            tone_map: tone_map_code(pipeline.tone_map.operator),
            edr_output: u32::from(edr_output),
            reserved0: 0,
            reserved1: 0,
            nits: [
                pipeline.source.nominal_peak_nits,
                pipeline.target.peak_nits,
                pipeline.source.reference_white_nits,
                pipeline.target.reference_white_nits,
            ],
            luma_coefficients: [luma.kr, luma.kg, luma.kb, 0.0],
            gamut_matrix_rows: pipeline.gamut_matrix().row4s(),
        }
    }
}

// Mirror of the `transfer_code` / `tone_map_code` mappings in macos.rs. Kept in sync
// with the Metal backend; the WGSL shader branches on these same integer codes.
fn transfer_code(transfer: TransferFunction) -> u32 {
    match transfer {
        TransferFunction::Srgb => 1,
        TransferFunction::Bt1886 => 2,
        TransferFunction::Pq => 3,
        TransferFunction::Hlg => 4,
        TransferFunction::Unknown => 0,
    }
}

fn tone_map_code(operator: ToneMapOperator) -> u32 {
    match operator {
        ToneMapOperator::Clip => 0,
        ToneMapOperator::Reinhard => 1,
        ToneMapOperator::Mobius => 2,
    }
}

fn overlay_has_planes(frame: &OverlayFrame) -> bool {
    !frame.subtitle_planes.is_empty()
}

/// Overlay quad uniforms, byte-compatible with the Metal `OverlayUniforms`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct OverlayUniforms {
    pub rect: [f32; 4],
    pub tex_rect: [f32; 4],
    pub viewport: [f32; 2],
    pub overlay_mode: u32,
    pub reserved0: u32,
    pub color: [f32; 4],
}

impl OverlayUniforms {
    /// A straight-RGBA subtitle plane placed at pixel `rect` within the viewport.
    fn rgba_plane(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        viewport_w: u32,
        viewport_h: u32,
    ) -> Self {
        Self {
            rect: [x as f32, y as f32, width as f32, height as f32],
            tex_rect: [0.0, 0.0, 1.0, 1.0],
            viewport: [viewport_w.max(1) as f32, viewport_h.max(1) as f32],
            overlay_mode: 0,
            reserved0: 0,
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// Lazily-built GPU objects for the NV12/P010 video pipeline, tied to the color
/// target format the pipeline was compiled for.
struct VideoPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
}

/// Lazily-built GPU objects for the overlay (subtitle/danmaku) compositing pass,
/// tied to the color target format it was compiled for.
struct OverlayPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
}

/// Per-plane GPU resources for one overlay draw. The texture and uniform buffer are
/// retained so the bind group stays valid for the duration of the render pass.
struct OverlayDraw {
    bind_group: wgpu::BindGroup,
    _texture: wgpu::Texture,
    _uniform: wgpu::Buffer,
}

/// The currently uploaded video frame: GPU plane textures plus the color uniforms
/// to render it. Retained so the presenter can re-present it across vsync ticks.
struct UploadedVideoFrame {
    luma: wgpu::Texture,
    chroma: wgpu::Texture,
    width: u32,
    height: u32,
    uniforms: VideoUniforms,
}

struct AttachedSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    handle: WgpuSurfaceHandle,
}

pub struct WgpuRenderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: Option<AttachedSurface>,
    video_pipeline: Option<VideoPipeline>,
    overlay_pipeline: Option<OverlayPipeline>,
    current_video: Option<UploadedVideoFrame>,
    stats: WgpuRendererStats,
}

/// Offscreen readback targets use a linear `Rgba8Unorm` format so a clear value of
/// `c` reads back as `round(c * 255)` with no transfer-function surprises.
const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl WgpuRenderer {
    pub fn new() -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|error| PlayerError::Renderer(format!("wgpu adapter request failed: {error}")))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("kuroko-wgpu-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|error| PlayerError::Renderer(format!("wgpu device request failed: {error}")))?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface: None,
            video_pipeline: None,
            overlay_pipeline: None,
            current_video: None,
            stats: WgpuRendererStats::default(),
        })
    }

    pub fn surface(&self) -> Option<WgpuSurfaceHandle> {
        self.surface.as_ref().map(|attached| attached.handle)
    }

    pub fn stats(&self) -> WgpuRendererStats {
        self.stats
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    /// Render a single clear pass into an offscreen `width`x`height` target and read
    /// the result back to host memory. This is the backend's headless test path: it
    /// needs no window or platform surface, so it runs under plain `cargo test`.
    pub fn clear_offscreen(
        &mut self,
        width: u32,
        height: u32,
        color: WgpuClearColor,
    ) -> Result<WgpuOffscreenReadback> {
        if width == 0 || height == 0 {
            return Err(PlayerError::Renderer(
                "offscreen target must have non-zero dimensions".to_string(),
            ));
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("kuroko-wgpu-offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kuroko-wgpu-offscreen-encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kuroko-wgpu-offscreen-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color.to_wgpu()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        self.queue.submit(Some(encoder.finish()));

        let rgba = self.read_back_rgba8(&texture, width, height)?;
        self.stats.offscreen_frames += 1;
        Ok(WgpuOffscreenReadback {
            width,
            height,
            rgba,
        })
    }

    /// Copy an RGBA8 texture into host memory, stripping the row padding that
    /// `copy_texture_to_buffer` requires (rows aligned to COPY_BYTES_PER_ROW_ALIGNMENT).
    fn read_back_rgba8(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Result<Vec<u8>> {
        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = (padded_bytes_per_row * height) as wgpu::BufferAddress;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kuroko-wgpu-readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kuroko-wgpu-readback-encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| PlayerError::Renderer(format!("wgpu device poll failed: {error}")))?;
        receiver
            .recv()
            .map_err(|_| PlayerError::Renderer("wgpu readback channel dropped".to_string()))?
            .map_err(|error| PlayerError::Renderer(format!("wgpu buffer map failed: {error}")))?;

        let mapped = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + unpadded_bytes_per_row as usize;
            rgba.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        readback.unmap();
        Ok(rgba)
    }

    /// Render a software-decoded NV12 frame through the WGSL video pipeline into an
    /// offscreen RGBA8 target and read it back. Mirrors the Metal `render_video_frame`
    /// path so results can be compared against the native backend.
    ///
    /// `luma` is `width * height` bytes (Y plane). `chroma` is the interleaved
    /// Cb/Cr plane at half resolution: `(width / 2) * (height / 2) * 2` bytes.
    pub fn render_nv12_offscreen(
        &mut self,
        width: u32,
        height: u32,
        luma: &[u8],
        chroma: &[u8],
        uniforms: VideoUniforms,
    ) -> Result<WgpuOffscreenReadback> {
        self.upload_nv12(width, height, luma, chroma, uniforms)?;
        self.render_current_offscreen(None)?
            .ok_or_else(|| PlayerError::Renderer("no current frame after upload".to_string()))
    }

    /// Upload tightly packed NV12 planes as the current video frame. `luma` is
    /// `width * height` bytes; `chroma` is the interleaved Cb/Cr plane at half
    /// resolution (`(width / 2) * (height / 2) * 2` bytes).
    pub fn upload_nv12(
        &mut self,
        width: u32,
        height: u32,
        luma: &[u8],
        chroma: &[u8],
        uniforms: VideoUniforms,
    ) -> Result<()> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(PlayerError::Renderer(
                "NV12 frame dimensions must be non-zero and even".to_string(),
            ));
        }
        let expected_luma = (width * height) as usize;
        let chroma_width = width / 2;
        let chroma_height = height / 2;
        let expected_chroma = (chroma_width * chroma_height * 2) as usize;
        if luma.len() != expected_luma {
            return Err(PlayerError::Renderer(format!(
                "NV12 luma plane is {} bytes, expected {expected_luma}",
                luma.len()
            )));
        }
        if chroma.len() != expected_chroma {
            return Err(PlayerError::Renderer(format!(
                "NV12 chroma plane is {} bytes, expected {expected_chroma}",
                chroma.len()
            )));
        }

        let luma_texture = self.create_plane_texture(
            "kuroko-wgpu-luma",
            width,
            height,
            wgpu::TextureFormat::R8Unorm,
            luma,
            width,
        );
        let chroma_texture = self.create_plane_texture(
            "kuroko-wgpu-chroma",
            chroma_width,
            chroma_height,
            wgpu::TextureFormat::Rg8Unorm,
            chroma,
            chroma_width * 2,
        );
        self.current_video = Some(UploadedVideoFrame {
            luma: luma_texture,
            chroma: chroma_texture,
            width,
            height,
            uniforms,
        });
        Ok(())
    }

    /// Render the current video frame (optionally compositing `overlay`) into an
    /// offscreen RGBA8 target and read it back. Returns `None` if no frame has been
    /// uploaded.
    pub fn render_current_offscreen(
        &mut self,
        overlay: Option<&OverlayFrame>,
    ) -> Result<Option<WgpuOffscreenReadback>> {
        if self.current_video.is_none() {
            return Ok(None);
        }
        self.ensure_video_pipeline(OFFSCREEN_FORMAT);
        if overlay.is_some_and(overlay_has_planes) {
            self.ensure_overlay_pipeline(OFFSCREEN_FORMAT);
        }
        let (width, height) = {
            let video = self.current_video.as_ref().expect("current video frame");
            (video.width, video.height)
        };
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("kuroko-wgpu-video-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        self.draw_current_video(&target_view, overlay)?;
        let rgba = self.read_back_rgba8(&target, width, height)?;
        self.stats.rendered_frames += 1;
        Ok(Some(WgpuOffscreenReadback {
            width,
            height,
            rgba,
        }))
    }

    /// Encode and submit a render pass drawing the current video frame into
    /// `target_view`. The caller must have uploaded a frame and the video pipeline
    /// must be initialized.
    fn draw_current_video(
        &self,
        target_view: &wgpu::TextureView,
        overlay: Option<&OverlayFrame>,
    ) -> Result<()> {
        let video = self
            .current_video
            .as_ref()
            .ok_or_else(|| PlayerError::Renderer("no current video frame".to_string()))?;
        let pipeline = self
            .video_pipeline
            .as_ref()
            .ok_or_else(|| PlayerError::Renderer("video pipeline not initialized".to_string()))?;

        let overlay_draws = match overlay {
            Some(frame) if overlay_has_planes(frame) => self.prepare_overlay_draws(frame)?,
            _ => Vec::new(),
        };

        let luma_view = video
            .luma
            .create_view(&wgpu::TextureViewDescriptor::default());
        let chroma_view = video
            .chroma
            .create_view(&wgpu::TextureViewDescriptor::default());
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kuroko-wgpu-video-uniforms"),
                contents: bytemuck::bytes_of(&video.uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kuroko-wgpu-video-bind-group"),
            layout: &pipeline.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&luma_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&chroma_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kuroko-wgpu-video-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kuroko-wgpu-video-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if !overlay_draws.is_empty() {
            let overlay_pipeline = self.overlay_pipeline.as_ref().ok_or_else(|| {
                PlayerError::Renderer("overlay pipeline not initialized".to_string())
            })?;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kuroko-wgpu-overlay-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Load to preserve the video plane, then alpha-blend overlays.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&overlay_pipeline.pipeline);
            for draw in &overlay_draws {
                pass.set_bind_group(0, &draw.bind_group, &[]);
                pass.draw(0..4, 0..1);
            }
        }

        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// Build per-plane GPU resources for the overlay's straight-RGBA subtitle planes.
    fn prepare_overlay_draws(&self, frame: &OverlayFrame) -> Result<Vec<OverlayDraw>> {
        let pipeline = self
            .overlay_pipeline
            .as_ref()
            .ok_or_else(|| PlayerError::Renderer("overlay pipeline not initialized".to_string()))?;
        let viewport_w = frame.viewport.width;
        let viewport_h = frame.viewport.height;
        let mut draws = Vec::with_capacity(frame.subtitle_planes.len());
        for plane in &frame.subtitle_planes {
            if plane.width == 0 || plane.height == 0 {
                continue;
            }
            let expected = plane.width as usize * plane.height as usize * 4;
            if plane.rgba.len() != expected {
                return Err(PlayerError::Renderer(format!(
                    "overlay subtitle plane has {} bytes, expected {expected} for {}x{} RGBA",
                    plane.rgba.len(),
                    plane.width,
                    plane.height
                )));
            }
            let texture = self.create_plane_texture(
                "kuroko-wgpu-overlay-plane",
                plane.width,
                plane.height,
                wgpu::TextureFormat::Rgba8Unorm,
                &plane.rgba,
                plane.width * 4,
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let uniforms = OverlayUniforms::rgba_plane(
                plane.x,
                plane.y,
                plane.width,
                plane.height,
                viewport_w,
                viewport_h,
            );
            let uniform = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("kuroko-wgpu-overlay-uniforms"),
                    contents: bytemuck::bytes_of(&uniforms),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("kuroko-wgpu-overlay-bind-group"),
                layout: &pipeline.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                    },
                ],
            });
            draws.push(OverlayDraw {
                bind_group,
                _texture: texture,
                _uniform: uniform,
            });
        }
        Ok(draws)
    }

    fn create_plane_texture(
        &self,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        data: &[u8],
        bytes_per_row: u32,
    ) -> wgpu::Texture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        texture
    }

    fn ensure_video_pipeline(&mut self, format: wgpu::TextureFormat) {
        // The render pipeline's color target format must match the render pass
        // attachment, so rebuild if the target format changed (offscreen Rgba8Unorm
        // vs the surface's format).
        if self
            .video_pipeline
            .as_ref()
            .is_some_and(|video| video.format == format)
        {
            return;
        }
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kuroko-wgpu-video-shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("wgpu_video.wgsl").into()),
            });
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bind_group_layout =
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("kuroko-wgpu-video-bgl"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        texture_entry(1),
                        texture_entry(2),
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kuroko-wgpu-video-layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("kuroko-wgpu-video-pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("kuroko_video_vertex"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("kuroko_video_fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache: None,
            });
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("kuroko-wgpu-video-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        self.video_pipeline = Some(VideoPipeline {
            pipeline,
            bind_group_layout,
            sampler,
            format,
        });
    }

    fn ensure_overlay_pipeline(&mut self, format: wgpu::TextureFormat) {
        if self
            .overlay_pipeline
            .as_ref()
            .is_some_and(|overlay| overlay.format == format)
        {
            return;
        }
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kuroko-wgpu-overlay-shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("wgpu_overlay.wgsl").into()),
            });
        let bind_group_layout =
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("kuroko-wgpu-overlay-bgl"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kuroko-wgpu-overlay-layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("kuroko-wgpu-overlay-pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("kuroko_overlay_vertex"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("kuroko_overlay_fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        // Straight-alpha blending, matching the Metal overlay pipeline.
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::SrcAlpha,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache: None,
            });
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("kuroko-wgpu-overlay-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        self.overlay_pipeline = Some(OverlayPipeline {
            pipeline,
            bind_group_layout,
            sampler,
            format,
        });
    }

    fn render_surface_clear(&mut self, color: WgpuClearColor) -> Result<()> {
        let Some(attached) = self.surface.as_ref() else {
            return Err(PlayerError::Renderer(
                "no wgpu surface attached".to_string(),
            ));
        };
        let frame = match attached.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            other => {
                return Err(PlayerError::Renderer(format!(
                    "wgpu surface acquire failed: {other:?}"
                )));
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kuroko-wgpu-surface-encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kuroko-wgpu-surface-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color.to_wgpu()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        self.stats.rendered_frames += 1;
        Ok(())
    }

    fn configure_surface(&mut self, width: u32, height: u32) {
        let Some(attached) = self.surface.as_mut() else {
            return;
        };
        attached.config.width = width.max(1);
        attached.config.height = height.max(1);
        attached.surface.configure(&self.device, &attached.config);
        self.stats.surface_width = attached.config.width;
        self.stats.surface_height = attached.config.height;
    }
}

impl RendererBackend for WgpuRenderer {
    fn attach_surface(&mut self, surface: PlatformSurface) -> Result<()> {
        let PlatformSurface::Wgpu(handle) = surface else {
            return Err(PlayerError::Renderer(
                "non-wgpu surface cannot be attached to WgpuRenderer".to_string(),
            ));
        };

        // SAFETY: `create_surface_unsafe` requires the raw handle to point at a live
        // CAMetalLayer that outlives the returned surface. The embedder owns the layer
        // for the lifetime of the attachment, mirroring the Metal renderer contract.
        let target = match handle.kind {
            WgpuSurfaceKind::MacOsCaMetalLayer => {
                wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(handle.raw_window as *mut c_void)
            }
            other => {
                return Err(PlayerError::Renderer(format!(
                    "wgpu surface kind {other:?} is not wired yet"
                )));
            }
        };
        let surface = unsafe { self.instance.create_surface_unsafe(target) }.map_err(|error| {
            PlayerError::Renderer(format!("wgpu surface creation failed: {error}"))
        })?;

        let caps = surface.get_capabilities(&self.adapter);
        // Prefer a non-sRGB format: the video shader already emits display-encoded
        // values for the SDR target, so an sRGB surface would double-encode gamma.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .unwrap_or_else(|| caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: handle.width.max(1),
            height: handle.height.max(1),
            present_mode: caps.present_modes[0],
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&self.device, &config);

        self.stats.surface_width = config.width;
        self.stats.surface_height = config.height;
        self.stats.attached = true;
        self.surface = Some(AttachedSurface {
            surface,
            config,
            handle,
        });
        Ok(())
    }

    fn detach_surface(&mut self) -> Result<()> {
        self.surface = None;
        self.stats.attached = false;
        Ok(())
    }

    fn resize_surface(&mut self, width: u32, height: u32, _scale: f64) -> Result<()> {
        if self.surface.is_none() {
            return Err(PlayerError::Renderer(
                "no wgpu surface attached".to_string(),
            ));
        }
        self.configure_surface(width, height);
        if let Some(attached) = self.surface.as_mut() {
            attached.handle.width = width;
            attached.handle.height = height;
        }
        Ok(())
    }

    fn render_test_frame(&mut self, time_seconds: f64) -> Result<()> {
        let color = WgpuClearColor::animated(time_seconds);
        if self.surface.is_some() {
            self.render_surface_clear(color)
        } else {
            // No surface: exercise the GPU path headlessly and count it as a frame.
            self.clear_offscreen(16, 16, color)?;
            self.stats.rendered_frames += 1;
            Ok(())
        }
    }

    fn upload_player_frame(&mut self, frame: &PlayerVideoFrame) -> Result<()> {
        // Software path: repack the decoded planes to NV12 and upload. A hardware
        // frame (e.g. VideoToolbox) has no CPU planes here; that needs the
        // per-platform zero-copy interop bridge (a later slice).
        let nv12 = frame.frame.to_nv12().ok_or_else(|| {
            PlayerError::Renderer(
                "wgpu: frame is not software 8-bit 4:2:0 (hardware frame or unsupported format)"
                    .to_string(),
            )
        })?;
        let source = SourceColorState::new(
            frame.frame.color_primaries(),
            frame.frame.transfer_function(),
        )
        .range(frame.frame.color_range())
        .matrix(frame.frame.matrix_coefficients())
        .hdr_metadata(frame.frame.hdr_metadata());
        let pipeline =
            VideoRenderPipeline::new(source, TargetColorState::sdr(ColorPrimaries::Bt709));
        let uniforms = VideoUniforms::from_pipeline(&pipeline, false, false);
        self.upload_nv12(nv12.width, nv12.height, &nv12.luma, &nv12.chroma, uniforms)
    }

    fn render_current_frame(&mut self, overlay: Option<&OverlayFrame>) -> Result<bool> {
        if self.current_video.is_none() {
            return Ok(false);
        }
        let Some(format) = self.surface.as_ref().map(|attached| attached.config.format) else {
            // No surface to present to (e.g. ticked before attach); the presenter
            // falls back to a test frame.
            return Ok(false);
        };
        self.ensure_video_pipeline(format);
        if overlay.is_some_and(overlay_has_planes) {
            self.ensure_overlay_pipeline(format);
        }
        let attached = self.surface.as_ref().expect("surface present");
        let frame = match attached.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            other => {
                return Err(PlayerError::Renderer(format!(
                    "wgpu surface acquire failed: {other:?}"
                )));
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.draw_current_video(&view, overlay)?;
        frame.present();
        self.stats.rendered_frames += 1;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::MetalSurfaceHandle;

    fn to_u8(component: f64) -> u8 {
        (component * 255.0).round() as u8
    }

    #[test]
    fn wgpu_renderer_clears_offscreen_target_to_expected_color() {
        let mut renderer = WgpuRenderer::new().unwrap();
        let color = WgpuClearColor::new(0.25, 0.5, 0.75, 1.0);

        let readback = renderer.clear_offscreen(4, 3, color).unwrap();

        assert_eq!(readback.width, 4);
        assert_eq!(readback.height, 3);
        assert_eq!(readback.rgba.len(), 4 * 3 * 4);
        let expected = [
            to_u8(color.red),
            to_u8(color.green),
            to_u8(color.blue),
            to_u8(color.alpha),
        ];
        for y in 0..readback.height {
            for x in 0..readback.width {
                let pixel = readback.pixel(x, y);
                // Allow a tolerance of 1 LSB for rounding differences across drivers.
                for channel in 0..4 {
                    let delta = (pixel[channel] as i16 - expected[channel] as i16).unsigned_abs();
                    assert!(
                        delta <= 1,
                        "pixel ({x},{y}) channel {channel} = {} expected ~{}",
                        pixel[channel],
                        expected[channel]
                    );
                }
            }
        }
        assert_eq!(renderer.stats().offscreen_frames, 1);
    }

    #[test]
    fn wgpu_renderer_render_test_frame_without_surface_uses_offscreen_path() {
        let mut renderer = WgpuRenderer::new().unwrap();

        renderer.render_test_frame(0.0).unwrap();

        let stats = renderer.stats();
        assert_eq!(stats.rendered_frames, 1);
        assert_eq!(stats.offscreen_frames, 1);
        assert!(!stats.attached);
    }

    #[test]
    fn wgpu_renderer_rejects_metal_surface() {
        let mut renderer = WgpuRenderer::new().unwrap();
        let result = renderer.attach_surface(PlatformSurface::Metal(MetalSurfaceHandle::new(
            42, 640, 360, 2.0,
        )));

        assert!(matches!(result, Err(PlayerError::Renderer(_))));
    }

    // --- Video pipeline parity oracle ---------------------------------------
    //
    // `reference_pixel` is a CPU port of the WGSL `kuroko_video_fragment` (which is
    // itself a port of the Metal `VIDEO_SHADER_SOURCE`). Asserting the GPU output
    // matches this reference proves the wgpu backend computes the same color math
    // as the native Metal renderer for the same uniforms.

    fn ref_pq_eotf(encoded: f32) -> f32 {
        let m1 = 0.1593017578125;
        let m2 = 78.84375;
        let c1 = 0.8359375;
        let c2 = 18.8515625;
        let c3 = 18.6875;
        let p = encoded.max(0.0).powf(1.0 / m2);
        let num = (p - c1).max(0.0);
        let den = (c2 - c3 * p).max(0.000001);
        (num / den).powf(1.0 / m1)
    }

    fn ref_transfer_to_source_linear(rgb: [f32; 3], u: &VideoUniforms) -> [f32; 3] {
        let rgb = rgb.map(|c| c.max(0.0));
        match u.source_transfer {
            3 => {
                let peak = u.nits[2].max(1.0);
                rgb.map(|c| ref_pq_eotf(c) * (10000.0 / peak))
            }
            1 => rgb.map(|c| c.powf(2.2)),
            2 => rgb.map(|c| c.powf(2.4)),
            _ => rgb,
        }
    }

    fn ref_gamut(rgb: [f32; 3], u: &VideoUniforms) -> [f32; 3] {
        let m = u.gamut_matrix_rows;
        [
            m[0][0] * rgb[0] + m[0][1] * rgb[1] + m[0][2] * rgb[2],
            m[1][0] * rgb[0] + m[1][1] * rgb[1] + m[1][2] * rgb[2],
            m[2][0] * rgb[0] + m[2][1] * rgb[1] + m[2][2] * rgb[2],
        ]
    }

    fn ref_tone_map(nits: [f32; 3], u: &VideoUniforms) -> [f32; 3] {
        let source_peak = u.nits[0].max(1.0);
        let target_peak = u.nits[1].max(1.0);
        let white = (source_peak / target_peak).max(1.0);
        let x = nits.map(|n| n.max(0.0) / target_peak);
        match u.tone_map {
            1 => {
                let white2 = white * white;
                x.map(|xi| target_peak * (xi * (1.0 + xi / white2) / (1.0 + xi)).clamp(0.0, 1.0))
            }
            2 => {
                let knee = 0.75;
                let denom = (white - knee).max(0.0001);
                x.map(|xi| {
                    let t = ((xi - knee) / denom).clamp(0.0, 1.0);
                    let shoulder = knee + (1.0 - knee) * (1.0 - (1.0 - t).powf(2.0));
                    let s = if xi >= knee { shoulder } else { xi };
                    target_peak * s
                })
            }
            _ => x.map(|xi| target_peak * xi.clamp(0.0, 1.0)),
        }
    }

    fn ref_output(rgb: [f32; 3], u: &VideoUniforms) -> [f32; 3] {
        if u.edr_output != 0 {
            return rgb.map(|c| c.max(0.0));
        }
        match u.target_transfer {
            1 => rgb.map(|c| c.max(0.0).powf(1.0 / 2.2)),
            2 => rgb.map(|c| c.max(0.0).powf(1.0 / 2.4)),
            _ => rgb,
        }
    }

    fn ref_final(rgb: [f32; 3], u: &VideoUniforms) -> [f32; 3] {
        if u.edr_output != 0 {
            let headroom = (u.nits[1].max(1.0) / u.nits[3].max(1.0)).max(1.0);
            rgb.map(|c| c.clamp(0.0, headroom))
        } else {
            rgb.map(|c| c.clamp(0.0, 1.0))
        }
    }

    fn reference_pixel(y: f32, cb: f32, cr: f32, u: &VideoUniforms) -> [f32; 3] {
        let (yy, cbcr) = if u.full_range != 0 {
            (y, [cb - 0.5, cr - 0.5])
        } else if u.is_p010 != 0 {
            (
                (y - 64.0 / 1023.0) * (1023.0 / 876.0),
                [
                    (cb - 512.0 / 1023.0) * (1023.0 / 896.0),
                    (cr - 512.0 / 1023.0) * (1023.0 / 896.0),
                ],
            )
        } else {
            (
                (y - 16.0 / 255.0) * (255.0 / 219.0),
                [
                    (cb - 128.0 / 255.0) * (255.0 / 224.0),
                    (cr - 128.0 / 255.0) * (255.0 / 224.0),
                ],
            )
        };
        let kr = u.luma_coefficients[0];
        let kg = u.luma_coefficients[1].max(0.000001);
        let kb = u.luma_coefficients[2];
        let r = yy + 2.0 * (1.0 - kr) * cbcr[1];
        let b = yy + 2.0 * (1.0 - kb) * cbcr[0];
        let g = (yy - kr * r - kb * b) / kg;
        let mut rgb = [r, g, b];
        rgb = ref_transfer_to_source_linear(rgb, u);
        rgb = ref_gamut(rgb, u);
        let srw = u.nits[2].max(1.0);
        rgb = rgb.map(|c| c.max(0.0) * srw);
        rgb = ref_tone_map(rgb, u);
        let trw = u.nits[3].max(1.0);
        rgb = rgb.map(|c| c.max(0.0) / trw);
        rgb = ref_output(rgb, u);
        ref_final(rgb, u)
    }

    fn build_solid_nv12(width: u32, height: u32, y: u8, cb: u8, cr: u8) -> (Vec<u8>, Vec<u8>) {
        let luma = vec![y; (width * height) as usize];
        let chroma_pixels = (width / 2) as usize * (height / 2) as usize;
        let mut chroma = Vec::with_capacity(chroma_pixels * 2);
        for _ in 0..chroma_pixels {
            chroma.push(cb);
            chroma.push(cr);
        }
        (luma, chroma)
    }

    #[test]
    fn wgpu_video_nv12_matches_cpu_reference() {
        let mut renderer = WgpuRenderer::new().unwrap();

        let sdr = VideoUniforms::from_pipeline(&VideoRenderPipeline::sdr_default(), false, false);

        // A full-range BT.709 identity configuration: linear in/out, clip tone map,
        // matched nits, identity gamut. Output should be the plain clamped YCbCr->RGB.
        let mut identity = sdr;
        identity.full_range = 1;
        identity.source_transfer = 0;
        identity.target_transfer = 0;
        identity.tone_map = 0;
        identity.nits = [100.0, 100.0, 100.0, 100.0];
        identity.luma_coefficients = [0.2126, 0.7152, 0.0722, 0.0];
        identity.gamut_matrix_rows = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];

        let samples = [
            (16u8, 128u8, 128u8),
            (128, 128, 128),
            (200, 90, 160),
            (80, 200, 64),
            (235, 128, 128),
        ];

        for uniforms in [sdr, identity] {
            for (y, cb, cr) in samples {
                let (luma, chroma) = build_solid_nv12(4, 4, y, cb, cr);
                let out = renderer
                    .render_nv12_offscreen(4, 4, &luma, &chroma, uniforms)
                    .unwrap();

                let expect = reference_pixel(
                    f32::from(y) / 255.0,
                    f32::from(cb) / 255.0,
                    f32::from(cr) / 255.0,
                    &uniforms,
                );
                let expected = [
                    to_u8(f64::from(expect[0])),
                    to_u8(f64::from(expect[1])),
                    to_u8(f64::from(expect[2])),
                    255,
                ];

                for py in 0..out.height {
                    for px in 0..out.width {
                        let pixel = out.pixel(px, py);
                        for channel in 0..4 {
                            let delta =
                                (pixel[channel] as i16 - expected[channel] as i16).unsigned_abs();
                            assert!(
                                delta <= 2,
                                "ycbcr ({y},{cb},{cr}) full_range={} pixel ch{channel} = {} expected ~{}",
                                uniforms.full_range,
                                pixel[channel],
                                expected[channel]
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn wgpu_renderer_is_usable_as_dyn_backend_and_reports_no_current_frame() {
        let mut renderer = WgpuRenderer::new().unwrap();
        // The presenter holds the backend as `Box<dyn RendererBackend>`; confirm the
        // wgpu renderer is object-safe through the trait and reports no current frame
        // so the presenter falls back to a test frame.
        let backend: &mut dyn RendererBackend = &mut renderer;
        assert!(!backend.render_current_frame(None).unwrap());
    }

    #[test]
    fn wgpu_video_rejects_wrong_plane_sizes() {
        let mut renderer = WgpuRenderer::new().unwrap();
        let uniforms =
            VideoUniforms::from_pipeline(&VideoRenderPipeline::sdr_default(), false, false);

        // Luma too short for a 4x4 frame.
        let result = renderer.render_nv12_offscreen(4, 4, &[0u8; 8], &[0u8; 8], uniforms);
        assert!(matches!(result, Err(PlayerError::Renderer(_))));

        // Odd dimensions are rejected.
        let result = renderer.render_nv12_offscreen(3, 4, &[0u8; 12], &[0u8; 4], uniforms);
        assert!(matches!(result, Err(PlayerError::Renderer(_))));
    }
}
