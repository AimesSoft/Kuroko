use std::ffi::c_void;
use std::mem;
use std::ptr::NonNull;

use crate::core::{PlayerError, Result};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CFRetained;
use objc2_core_foundation::CGSize;
use objc2_core_video::kCVReturnSuccess;
use objc2_core_video::{
    CVImageBuffer, CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture,
};
use objc2_core_video::{CVPixelBuffer, CVPixelBufferGetHeight, CVPixelBufferGetHeightOfPlane};
use objc2_core_video::{
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetPlaneCount, CVPixelBufferGetWidth,
};
use objc2_core_video::{
    CVPixelBufferGetWidthOfPlane, kCVPixelFormatType_420YpCbCr10BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange,
};
use objc2_core_video::{
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLClearColor, MTLCreateSystemDefaultDevice, MTLLoadAction,
    MTLPixelFormat, MTLRegion, MTLResourceOptions, MTLStoreAction, MTLTextureDescriptor,
    MTLTextureUsage,
};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLDevice, MTLDrawable,
    MTLRenderPassDescriptor, MTLTexture,
};
use objc2_metal::{
    MTLLibrary, MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState,
};
use objc2_metal::{MTLSamplerDescriptor, MTLSamplerMinMagFilter, MTLSamplerState};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};

use crate::renderer::metal::{
    ClearColor, ImportedVideoFormat, ImportedVideoFrameInfo, ImportedVideoPlaneInfo,
    MetalRendererStats, OverlayRenderFrame, PreparedOverlayFrameInfo, VideoFrameTextureSource,
    VideoRenderFrame, fourcc_string,
};
use crate::renderer::pipeline::{ColorRange, ToneMapOperator};

const CV_PIXEL_FORMAT_420_YP_CB_CR10_BI_PLANAR_VIDEO_RANGE: u32 =
    kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange;
const CV_PIXEL_FORMAT_420_YP_CB_CR10_BI_PLANAR_FULL_RANGE: u32 =
    kCVPixelFormatType_420YpCbCr10BiPlanarFullRange;
const CV_PIXEL_FORMAT_420_YP_CB_CR8_BI_PLANAR_VIDEO_RANGE: u32 =
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange;
const CV_PIXEL_FORMAT_420_YP_CB_CR8_BI_PLANAR_FULL_RANGE: u32 =
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;

pub struct ImportedVideoFrameTextures {
    #[allow(dead_code)]
    source_pixel_buffer: CFRetained<CVPixelBuffer>,
    planes: Vec<ImportedVideoPlaneTexture>,
}

impl ImportedVideoFrameTextures {
    pub fn plane_count(&self) -> usize {
        self.planes.len()
    }
}

struct ImportedVideoPlaneTexture {
    #[allow(dead_code)]
    cv_texture: CFRetained<CVMetalTexture>,
    #[allow(dead_code)]
    metal_texture: Retained<ProtocolObject<dyn MTLTexture>>,
}

impl ImportedVideoFrameTextures {
    fn luma_texture(&self) -> Option<&ProtocolObject<dyn MTLTexture>> {
        self.planes
            .first()
            .map(|plane| plane.metal_texture.as_ref())
    }

    fn chroma_texture(&self) -> Option<&ProtocolObject<dyn MTLTexture>> {
        self.planes.get(1).map(|plane| plane.metal_texture.as_ref())
    }
}

pub struct ImportedVideoFrameResult {
    pub info: ImportedVideoFrameInfo,
    pub textures: ImportedVideoFrameTextures,
}

pub struct MetalRendererImpl {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    layer: Option<Retained<CAMetalLayer>>,
    texture_cache: Option<CFRetained<CVMetalTextureCache>>,
    video_pipeline: Option<Retained<ProtocolObject<dyn MTLRenderPipelineState>>>,
    overlay_pipeline: Option<Retained<ProtocolObject<dyn MTLRenderPipelineState>>>,
    video_sampler: Option<Retained<ProtocolObject<dyn MTLSamplerState>>>,
    stats: MetalRendererStats,
}

impl MetalRendererImpl {
    pub fn new() -> Result<Self> {
        let device = MTLCreateSystemDefaultDevice().ok_or_else(|| {
            PlayerError::Renderer("MTLCreateSystemDefaultDevice returned nil".to_string())
        })?;
        let queue = device
            .newCommandQueue()
            .ok_or_else(|| PlayerError::Renderer("newCommandQueue returned nil".to_string()))?;
        Ok(Self {
            device,
            queue,
            layer: None,
            texture_cache: None,
            video_pipeline: None,
            overlay_pipeline: None,
            video_sampler: None,
            stats: MetalRendererStats::default(),
        })
    }

    pub unsafe fn attach_raw_layer(
        &mut self,
        layer: *mut c_void,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Result<()> {
        if layer.is_null() {
            return Err(PlayerError::Renderer(
                "cannot attach null CAMetalLayer".to_string(),
            ));
        }
        let layer: Retained<CAMetalLayer> = unsafe { Retained::retain(layer.cast()) }
            .ok_or_else(|| PlayerError::Renderer("failed to retain CAMetalLayer".to_string()))?;
        layer.setDevice(Some(&*self.device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        self.layer = Some(layer);
        self.resize_surface(width, height, scale);
        Ok(())
    }

    pub fn detach_surface(&mut self) {
        self.layer = None;
    }

    pub fn resize_surface(&mut self, width: u32, height: u32, scale: f64) {
        let Some(layer) = &self.layer else {
            return;
        };
        let drawable_width = (width as f64 * scale).max(1.0);
        let drawable_height = (height as f64 * scale).max(1.0);
        let size = CGSize::new(drawable_width, drawable_height);
        layer.setDrawableSize(size);
        self.stats.drawable_width = drawable_width.round() as u32;
        self.stats.drawable_height = drawable_height.round() as u32;
    }

    pub fn stats(&self) -> MetalRendererStats {
        self.stats
    }

    pub fn record_prepared_overlay_frame(&mut self, info: PreparedOverlayFrameInfo) {
        self.stats.prepared_overlay_frames += 1;
        self.stats.prepared_overlay_subtitle_planes += info.subtitle_planes as u64;
        self.stats.prepared_overlay_danmaku_boxes += info.danmaku_boxes as u64;
    }

    pub fn render_clear(&mut self, color: ClearColor) -> Result<()> {
        let Some(layer) = &self.layer else {
            return Err(PlayerError::Renderer(
                "no CAMetalLayer attached".to_string(),
            ));
        };

        unsafe {
            let Some(drawable): Option<Retained<ProtocolObject<dyn CAMetalDrawable>>> =
                layer.nextDrawable()
            else {
                return Err(PlayerError::Renderer(
                    "CAMetalLayer nextDrawable returned nil".to_string(),
                ));
            };

            let descriptor = MTLRenderPassDescriptor::new();
            let attachments = descriptor.colorAttachments();
            let attachment = attachments.objectAtIndexedSubscript(0);
            let texture = drawable.texture();
            attachment.setTexture(Some(&*texture));
            attachment.setLoadAction(MTLLoadAction::Clear);
            attachment.setStoreAction(MTLStoreAction::Store);
            attachment.setClearColor(MTLClearColor {
                red: color.red,
                green: color.green,
                blue: color.blue,
                alpha: color.alpha,
            });

            let Some(command_buffer) = self.queue.commandBuffer() else {
                return Err(PlayerError::Renderer(
                    "commandBuffer returned nil".to_string(),
                ));
            };
            let Some(encoder) = command_buffer.renderCommandEncoderWithDescriptor(&descriptor)
            else {
                return Err(PlayerError::Renderer(
                    "renderCommandEncoderWithDescriptor returned nil".to_string(),
                ));
            };
            encoder.endEncoding();
            let drawable_ref: &ProtocolObject<dyn MTLDrawable> =
                ProtocolObject::from_ref(&*drawable);
            command_buffer.presentDrawable(drawable_ref);
            command_buffer.commit();
        }

        self.stats.rendered_frames += 1;

        Ok(())
    }

    pub fn render_video_frame(&mut self, frame: VideoRenderFrame<'_>) -> Result<()> {
        self.render_video_frame_inner(frame, None)
    }

    pub fn render_video_frame_with_overlay(
        &mut self,
        frame: VideoRenderFrame<'_>,
        overlay: OverlayRenderFrame<'_>,
    ) -> Result<()> {
        self.render_video_frame_inner(frame, Some(overlay))
    }

    pub fn render_overlay_frame(&mut self, overlay: OverlayRenderFrame<'_>) -> Result<()> {
        let Some(layer) = &self.layer else {
            return Err(PlayerError::Renderer(
                "no CAMetalLayer attached".to_string(),
            ));
        };

        unsafe {
            let Some(drawable): Option<Retained<ProtocolObject<dyn CAMetalDrawable>>> =
                layer.nextDrawable()
            else {
                return Err(PlayerError::Renderer(
                    "CAMetalLayer nextDrawable returned nil".to_string(),
                ));
            };

            let descriptor = MTLRenderPassDescriptor::new();
            let attachments = descriptor.colorAttachments();
            let attachment = attachments.objectAtIndexedSubscript(0);
            let texture = drawable.texture();
            attachment.setTexture(Some(&*texture));
            attachment.setLoadAction(MTLLoadAction::Clear);
            attachment.setStoreAction(MTLStoreAction::Store);
            attachment.setClearColor(MTLClearColor {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
            });

            let Some(command_buffer) = self.queue.commandBuffer() else {
                return Err(PlayerError::Renderer(
                    "commandBuffer returned nil".to_string(),
                ));
            };
            let Some(encoder) = command_buffer.renderCommandEncoderWithDescriptor(&descriptor)
            else {
                return Err(PlayerError::Renderer(
                    "renderCommandEncoderWithDescriptor returned nil".to_string(),
                ));
            };
            self.draw_overlay_planes(&encoder, overlay)?;
            encoder.endEncoding();
            let drawable_ref: &ProtocolObject<dyn MTLDrawable> =
                ProtocolObject::from_ref(&*drawable);
            command_buffer.presentDrawable(drawable_ref);
            command_buffer.commit();
        }

        self.stats.rendered_frames += 1;
        Ok(())
    }

    fn render_video_frame_inner(
        &mut self,
        frame: VideoRenderFrame<'_>,
        overlay: Option<OverlayRenderFrame<'_>>,
    ) -> Result<()> {
        let Some(layer) = &self.layer else {
            return Err(PlayerError::Renderer(
                "no CAMetalLayer attached".to_string(),
            ));
        };

        let Some(textures) = frame.frame.inner.as_ref() else {
            return Err(PlayerError::Renderer(
                "imported video frame has no Metal textures".to_string(),
            ));
        };
        let Some(luma) = textures.luma_texture() else {
            return Err(PlayerError::Renderer(
                "imported video frame has no luma plane".to_string(),
            ));
        };
        let Some(chroma) = textures.chroma_texture() else {
            return Err(PlayerError::Renderer(
                "imported video frame has no chroma plane".to_string(),
            ));
        };

        unsafe {
            let Some(drawable): Option<Retained<ProtocolObject<dyn CAMetalDrawable>>> =
                layer.nextDrawable()
            else {
                return Err(PlayerError::Renderer(
                    "CAMetalLayer nextDrawable returned nil".to_string(),
                ));
            };

            let descriptor = MTLRenderPassDescriptor::new();
            let attachments = descriptor.colorAttachments();
            let attachment = attachments.objectAtIndexedSubscript(0);
            let texture = drawable.texture();
            attachment.setTexture(Some(&*texture));
            attachment.setLoadAction(MTLLoadAction::Clear);
            attachment.setStoreAction(MTLStoreAction::Store);
            attachment.setClearColor(MTLClearColor {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
            });

            let pipeline = self.video_pipeline_state()?;
            let sampler = self.video_sampler_state()?;
            let Some(command_buffer) = self.queue.commandBuffer() else {
                return Err(PlayerError::Renderer(
                    "commandBuffer returned nil".to_string(),
                ));
            };
            let Some(encoder) = command_buffer.renderCommandEncoderWithDescriptor(&descriptor)
            else {
                return Err(PlayerError::Renderer(
                    "renderCommandEncoderWithDescriptor returned nil".to_string(),
                ));
            };

            let uniforms = VideoUniforms {
                is_p010: matches!(frame.frame.info.format, ImportedVideoFormat::P010) as u32,
                full_range: matches!(frame.pipeline.source.range, ColorRange::Full) as u32,
                source_transfer: transfer_code(frame.pipeline.source.transfer),
                target_transfer: transfer_code(frame.pipeline.target.transfer),
                tone_map: tone_map_code(frame.pipeline.tone_map.operator),
                source_peak_nits: frame.pipeline.source.nominal_peak_nits,
                target_peak_nits: frame.pipeline.target.peak_nits,
                luma_coefficients: luma_coefficients(frame.pipeline.luma_coefficients()),
            };
            encoder.setRenderPipelineState(&pipeline);
            encoder.setFragmentTexture_atIndex(Some(luma), 0);
            encoder.setFragmentTexture_atIndex(Some(chroma), 1);
            encoder.setFragmentSamplerState_atIndex(Some(&sampler), 0);
            encoder.setFragmentBytes_length_atIndex(
                NonNull::new(
                    (&uniforms as *const VideoUniforms)
                        .cast::<c_void>()
                        .cast_mut(),
                )
                .expect("uniform pointer is non-null"),
                mem::size_of::<VideoUniforms>(),
                0,
            );
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);

            if let Some(overlay) = overlay {
                self.draw_overlay_planes(&encoder, overlay)?;
            }

            encoder.endEncoding();
            let drawable_ref: &ProtocolObject<dyn MTLDrawable> =
                ProtocolObject::from_ref(&*drawable);
            command_buffer.presentDrawable(drawable_ref);
            command_buffer.commit();
        }

        self.stats.rendered_frames += 1;

        Ok(())
    }

    fn draw_overlay_planes(
        &mut self,
        encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
        overlay: OverlayRenderFrame<'_>,
    ) -> Result<()> {
        let _ = crate::renderer::metal::inspect_overlay_frame(overlay.frame)?;
        if overlay.frame.subtitle_planes.is_empty() {
            return Ok(());
        }

        let pipeline = self.overlay_pipeline_state()?;
        let sampler = self.video_sampler_state()?;
        let viewport = overlay.frame.viewport;
        for plane in &overlay.frame.subtitle_planes {
            let texture = self.create_overlay_texture(
                plane.width as usize,
                plane.height as usize,
                &plane.rgba,
            )?;
            let uniforms = OverlayUniforms::from_plane(
                plane.x,
                plane.y,
                plane.width,
                plane.height,
                viewport.width,
                viewport.height,
            );
            unsafe {
                encoder.setRenderPipelineState(&pipeline);
                encoder.setFragmentTexture_atIndex(Some(&*texture), 0);
                encoder.setFragmentSamplerState_atIndex(Some(&sampler), 0);
                encoder.setVertexBytes_length_atIndex(
                    NonNull::new(
                        (&uniforms as *const OverlayUniforms)
                            .cast::<c_void>()
                            .cast_mut(),
                    )
                    .expect("overlay uniform pointer is non-null"),
                    mem::size_of::<OverlayUniforms>(),
                    0,
                );
                encoder.drawPrimitives_vertexStart_vertexCount(
                    MTLPrimitiveType::TriangleStrip,
                    0,
                    4,
                );
            }
        }
        Ok(())
    }

    fn create_overlay_texture(
        &self,
        width: usize,
        height: usize,
        rgba: &[u8],
    ) -> Result<Retained<ProtocolObject<dyn MTLTexture>>> {
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::RGBA8Unorm,
                width,
                height,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead);
        descriptor.setResourceOptions(MTLResourceOptions::StorageModeShared);
        let texture = self
            .device
            .newTextureWithDescriptor(&descriptor)
            .ok_or_else(|| {
                PlayerError::Renderer("newTextureWithDescriptor returned nil".to_string())
            })?;
        let region = MTLRegion {
            origin: objc2_metal::MTLOrigin { x: 0, y: 0, z: 0 },
            size: objc2_metal::MTLSize {
                width,
                height,
                depth: 1,
            },
        };
        unsafe {
            texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                region,
                0,
                NonNull::new(rgba.as_ptr().cast::<c_void>().cast_mut())
                    .expect("overlay rgba pointer is non-null"),
                width * 4,
            );
        }
        Ok(texture)
    }

    pub unsafe fn import_video_frame_textures(
        &mut self,
        source: VideoFrameTextureSource,
    ) -> Result<ImportedVideoFrameResult> {
        if source.raw_pixel_buffer.is_null() {
            return Err(PlayerError::Renderer(
                "cannot import null CVPixelBuffer".to_string(),
            ));
        }

        let pixel_buffer = unsafe { &*(source.raw_pixel_buffer.cast::<CVPixelBuffer>()) };
        let retained_pixel_buffer = unsafe {
            CFRetained::retain(
                NonNull::new(source.raw_pixel_buffer.cast::<CVPixelBuffer>())
                    .expect("checked non-null CVPixelBuffer"),
            )
        };
        let pixel_format = CVPixelBufferGetPixelFormatType(pixel_buffer);
        let mapping =
            PixelBufferMapping::from_core_video_format(pixel_format).ok_or_else(|| {
                PlayerError::Renderer(format!(
                    "unsupported CVPixelBuffer format {}",
                    fourcc_string(pixel_format)
                ))
            })?;
        let plane_count = CVPixelBufferGetPlaneCount(pixel_buffer);
        if plane_count < 2 {
            return Err(PlayerError::Renderer(format!(
                "expected at least 2 planes for {}, got {plane_count}",
                fourcc_string(pixel_format)
            )));
        }

        let cache = self.texture_cache()?;
        let image_buffer = unsafe { &*(source.raw_pixel_buffer.cast::<CVImageBuffer>()) };
        let mut imported_textures = Vec::with_capacity(2);
        let mut planes = Vec::with_capacity(2);

        for plane in mapping.planes {
            let width = CVPixelBufferGetWidthOfPlane(pixel_buffer, plane.index);
            let height = CVPixelBufferGetHeightOfPlane(pixel_buffer, plane.index);
            let texture = create_plane_texture(
                cache,
                image_buffer,
                plane.pixel_format,
                width,
                height,
                plane.index,
            )?;
            let Some(metal_texture) = CVMetalTextureGetTexture(&texture) else {
                return Err(PlayerError::Renderer(format!(
                    "CVMetalTextureGetTexture returned nil for plane {}",
                    plane.index
                )));
            };
            planes.push(ImportedVideoPlaneInfo {
                index: plane.index,
                width: metal_texture.width(),
                height: metal_texture.height(),
                metal_pixel_format: plane.name,
            });
            imported_textures.push(ImportedVideoPlaneTexture {
                cv_texture: texture,
                metal_texture,
            });
        }

        let info = ImportedVideoFrameInfo {
            width: CVPixelBufferGetWidth(pixel_buffer).max(source.width as usize),
            height: CVPixelBufferGetHeight(pixel_buffer).max(source.height as usize),
            pixel_format,
            pixel_format_fourcc: fourcc_string(pixel_format),
            format: mapping.format,
            full_range: mapping.full_range,
            color_range: if mapping.full_range {
                ColorRange::Full
            } else {
                ColorRange::Limited
            },
            planes,
        };

        Ok(ImportedVideoFrameResult {
            info,
            textures: ImportedVideoFrameTextures {
                source_pixel_buffer: retained_pixel_buffer,
                planes: imported_textures,
            },
        })
    }

    fn texture_cache(&mut self) -> Result<&CVMetalTextureCache> {
        if self.texture_cache.is_none() {
            let mut raw_cache: *mut CVMetalTextureCache = std::ptr::null_mut();
            let status = unsafe {
                CVMetalTextureCache::create(
                    None,
                    None,
                    &self.device,
                    None,
                    NonNull::new(&mut raw_cache as *mut *mut CVMetalTextureCache)
                        .expect("stack cache pointer is non-null"),
                )
            };
            if status != kCVReturnSuccess {
                return Err(PlayerError::Renderer(format!(
                    "CVMetalTextureCacheCreate failed: {status}"
                )));
            }
            let raw_cache = NonNull::new(raw_cache).ok_or_else(|| {
                PlayerError::Renderer("CVMetalTextureCacheCreate returned null cache".to_string())
            })?;
            self.texture_cache = Some(unsafe { CFRetained::from_raw(raw_cache) });
        }
        Ok(self.texture_cache.as_deref().expect("texture cache exists"))
    }

    fn video_pipeline_state(
        &mut self,
    ) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>> {
        if self.video_pipeline.is_none() {
            let library = self
                .device
                .newLibraryWithSource_options_error(&NSString::from_str(VIDEO_SHADER_SOURCE), None)
                .map_err(|error| {
                    PlayerError::Renderer(format!(
                        "Metal video shader compile failed: {}",
                        error.localizedDescription()
                    ))
                })?;
            let vertex = library
                .newFunctionWithName(&NSString::from_str("kuroko_video_vertex"))
                .ok_or_else(|| {
                    PlayerError::Renderer("Metal shader missing kuroko_video_vertex".to_string())
                })?;
            let fragment = library
                .newFunctionWithName(&NSString::from_str("kuroko_video_fragment"))
                .ok_or_else(|| {
                    PlayerError::Renderer("Metal shader missing kuroko_video_fragment".to_string())
                })?;
            let descriptor = MTLRenderPipelineDescriptor::new();
            descriptor.setLabel(Some(&NSString::from_str("Kuroko Video Pipeline")));
            descriptor.setVertexFunction(Some(&*vertex));
            descriptor.setFragmentFunction(Some(&*fragment));
            let attachments = descriptor.colorAttachments();
            let attachment = unsafe { attachments.objectAtIndexedSubscript(0) };
            attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            let pipeline = self
                .device
                .newRenderPipelineStateWithDescriptor_error(&descriptor)
                .map_err(|error| {
                    PlayerError::Renderer(format!(
                        "Metal video pipeline create failed: {}",
                        error.localizedDescription()
                    ))
                })?;
            self.video_pipeline = Some(pipeline);
        }
        Ok(self
            .video_pipeline
            .as_ref()
            .expect("video pipeline exists")
            .clone())
    }

    fn video_sampler_state(&mut self) -> Result<Retained<ProtocolObject<dyn MTLSamplerState>>> {
        if self.video_sampler.is_none() {
            let descriptor = MTLSamplerDescriptor::new();
            descriptor.setMinFilter(MTLSamplerMinMagFilter::Linear);
            descriptor.setMagFilter(MTLSamplerMinMagFilter::Linear);
            let sampler = self
                .device
                .newSamplerStateWithDescriptor(&descriptor)
                .ok_or_else(|| {
                    PlayerError::Renderer("newSamplerStateWithDescriptor returned nil".to_string())
                })?;
            self.video_sampler = Some(sampler);
        }
        Ok(self
            .video_sampler
            .as_ref()
            .expect("video sampler exists")
            .clone())
    }

    fn overlay_pipeline_state(
        &mut self,
    ) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>> {
        if self.overlay_pipeline.is_none() {
            let library = self
                .device
                .newLibraryWithSource_options_error(&NSString::from_str(VIDEO_SHADER_SOURCE), None)
                .map_err(|error| {
                    PlayerError::Renderer(format!(
                        "Metal overlay shader compile failed: {}",
                        error.localizedDescription()
                    ))
                })?;
            let vertex = library
                .newFunctionWithName(&NSString::from_str("kuroko_overlay_vertex"))
                .ok_or_else(|| {
                    PlayerError::Renderer("Metal shader missing kuroko_overlay_vertex".to_string())
                })?;
            let fragment = library
                .newFunctionWithName(&NSString::from_str("kuroko_overlay_fragment"))
                .ok_or_else(|| {
                    PlayerError::Renderer(
                        "Metal shader missing kuroko_overlay_fragment".to_string(),
                    )
                })?;
            let descriptor = MTLRenderPipelineDescriptor::new();
            descriptor.setLabel(Some(&NSString::from_str("Kuroko Overlay Pipeline")));
            descriptor.setVertexFunction(Some(&*vertex));
            descriptor.setFragmentFunction(Some(&*fragment));
            let attachments = descriptor.colorAttachments();
            let attachment = unsafe { attachments.objectAtIndexedSubscript(0) };
            attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            attachment.setBlendingEnabled(true);
            attachment.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
            attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
            attachment.setRgbBlendOperation(MTLBlendOperation::Add);
            attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
            attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
            attachment.setAlphaBlendOperation(MTLBlendOperation::Add);
            let pipeline = self
                .device
                .newRenderPipelineStateWithDescriptor_error(&descriptor)
                .map_err(|error| {
                    PlayerError::Renderer(format!(
                        "Metal overlay pipeline create failed: {}",
                        error.localizedDescription()
                    ))
                })?;
            self.overlay_pipeline = Some(pipeline);
        }
        Ok(self
            .overlay_pipeline
            .as_ref()
            .expect("overlay pipeline exists")
            .clone())
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VideoUniforms {
    is_p010: u32,
    full_range: u32,
    source_transfer: u32,
    target_transfer: u32,
    tone_map: u32,
    source_peak_nits: f32,
    target_peak_nits: f32,
    luma_coefficients: [f32; 4],
}

fn transfer_code(transfer: crate::core::TransferFunction) -> u32 {
    match transfer {
        crate::core::TransferFunction::Srgb => 1,
        crate::core::TransferFunction::Bt1886 => 2,
        crate::core::TransferFunction::Pq => 3,
        crate::core::TransferFunction::Hlg => 4,
        crate::core::TransferFunction::Unknown => 0,
    }
}

fn tone_map_code(operator: ToneMapOperator) -> u32 {
    match operator {
        ToneMapOperator::Clip => 0,
        ToneMapOperator::Reinhard => 1,
        ToneMapOperator::Mobius => 2,
    }
}

fn luma_coefficients(coeffs: crate::renderer::pipeline::LumaCoefficients) -> [f32; 4] {
    [coeffs.kr, coeffs.kg, coeffs.kb, 0.0]
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct OverlayUniforms {
    rect: [f32; 4],
    viewport: [f32; 2],
    _padding: [f32; 2],
}

impl OverlayUniforms {
    fn from_plane(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Self {
        Self {
            rect: [x as f32, y as f32, width as f32, height as f32],
            viewport: [viewport_width.max(1) as f32, viewport_height.max(1) as f32],
            _padding: [0.0; 2],
        }
    }
}

const VIDEO_SHADER_SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct VertexOut {
    float4 position [[position]];
    float2 tex_coord;
};

struct VideoUniforms {
    uint is_p010;
    uint full_range;
    uint source_transfer;
    uint target_transfer;
    uint tone_map;
    float source_peak_nits;
    float target_peak_nits;
    float4 luma_coefficients;
};

float pq_eotf(float encoded) {
    constexpr float m1 = 0.1593017578125;
    constexpr float m2 = 78.84375;
    constexpr float c1 = 0.8359375;
    constexpr float c2 = 18.8515625;
    constexpr float c3 = 18.6875;
    float p = pow(max(encoded, 0.0), 1.0 / m2);
    float num = max(p - c1, 0.0);
    float den = max(c2 - c3 * p, 0.000001);
    return pow(num / den, 1.0 / m1);
}

float3 transfer_to_linear(float3 rgb, constant VideoUniforms& uniforms) {
    rgb = max(rgb, float3(0.0));
    if (uniforms.source_transfer == 3) {
        return float3(pq_eotf(rgb.r), pq_eotf(rgb.g), pq_eotf(rgb.b));
    }
    if (uniforms.source_transfer == 1) {
        return pow(rgb, float3(2.2));
    }
    if (uniforms.source_transfer == 2) {
        return pow(rgb, float3(2.4));
    }
    return rgb;
}

float3 tone_map_rgb(float3 rgb, constant VideoUniforms& uniforms) {
    float source_peak = max(uniforms.source_peak_nits, 1.0);
    float target_peak = max(uniforms.target_peak_nits, 1.0);
    float scale = source_peak / target_peak;
    float3 scaled = rgb * scale;
    if (uniforms.tone_map == 1) {
        return scaled / (float3(1.0) + scaled);
    }
    if (uniforms.tone_map == 2) {
        constexpr float knee = 0.75;
        float3 high = scaled / (float3(1.0) + scaled);
        return mix(scaled, high, smoothstep(knee, 1.0, max(max(scaled.r, scaled.g), scaled.b)));
    }
    return scaled;
}

float3 linear_to_output(float3 rgb, constant VideoUniforms& uniforms) {
    if (uniforms.target_transfer == 1) {
        return pow(max(rgb, float3(0.0)), float3(1.0 / 2.2));
    }
    if (uniforms.target_transfer == 2) {
        return pow(max(rgb, float3(0.0)), float3(1.0 / 2.4));
    }
    return rgb;
}

struct RangeExpandedYCbCr {
    float y;
    float2 cbcr;
};

RangeExpandedYCbCr expand_ycbcr_range(float y, float2 cbcr, constant VideoUniforms& uniforms) {
    if (uniforms.full_range != 0) {
        return RangeExpandedYCbCr { y, cbcr - float2(0.5) };
    }

    if (uniforms.is_p010 != 0) {
        y = (y - (64.0 / 1023.0)) * (1023.0 / 876.0);
        cbcr = (cbcr - float2(512.0 / 1023.0)) * (1023.0 / 896.0);
        return RangeExpandedYCbCr { y, cbcr };
    }

    y = (y - (16.0 / 255.0)) * (255.0 / 219.0);
    cbcr = (cbcr - float2(128.0 / 255.0)) * (255.0 / 224.0);
    return RangeExpandedYCbCr { y, cbcr };
}

vertex VertexOut kuroko_video_vertex(uint vertex_id [[vertex_id]]) {
    constexpr float2 positions[3] = {
        float2(-1.0, -1.0),
        float2( 3.0, -1.0),
        float2(-1.0,  3.0),
    };
    constexpr float2 tex_coords[3] = {
        float2(0.0, 1.0),
        float2(2.0, 1.0),
        float2(0.0, -1.0),
    };
    VertexOut out;
    out.position = float4(positions[vertex_id], 0.0, 1.0);
    out.tex_coord = tex_coords[vertex_id];
    return out;
}

fragment float4 kuroko_video_fragment(
    VertexOut in [[stage_in]],
    texture2d<float, access::sample> luma_texture [[texture(0)]],
    texture2d<float, access::sample> chroma_texture [[texture(1)]],
    sampler video_sampler [[sampler(0)]],
    constant VideoUniforms& uniforms [[buffer(0)]]) {
    float y = luma_texture.sample(video_sampler, in.tex_coord).r;
    float2 cbcr = chroma_texture.sample(video_sampler, in.tex_coord).rg;
    RangeExpandedYCbCr expanded = expand_ycbcr_range(y, cbcr, uniforms);
    y = expanded.y;
    cbcr = expanded.cbcr;

    float kr = uniforms.luma_coefficients.x;
    float kg = max(uniforms.luma_coefficients.y, 0.000001);
    float kb = uniforms.luma_coefficients.z;
    float3 rgb;
    rgb.r = y + 2.0 * (1.0 - kr) * cbcr.y;
    rgb.b = y + 2.0 * (1.0 - kb) * cbcr.x;
    rgb.g = (y - kr * rgb.r - kb * rgb.b) / kg;
    rgb = transfer_to_linear(rgb, uniforms);
    rgb = tone_map_rgb(rgb, uniforms);
    rgb = linear_to_output(rgb, uniforms);
    return float4(clamp(rgb, 0.0, 1.0), 1.0);
}

struct OverlayUniforms {
    float4 rect;
    float2 viewport;
    float2 padding;
};

vertex VertexOut kuroko_overlay_vertex(
    uint vertex_id [[vertex_id]],
    constant OverlayUniforms& uniforms [[buffer(0)]]) {
    constexpr float2 unit_positions[4] = {
        float2(0.0, 0.0),
        float2(1.0, 0.0),
        float2(0.0, 1.0),
        float2(1.0, 1.0),
    };
    constexpr float2 tex_coords[4] = {
        float2(0.0, 0.0),
        float2(1.0, 0.0),
        float2(0.0, 1.0),
        float2(1.0, 1.0),
    };

    float2 pixel = uniforms.rect.xy + unit_positions[vertex_id] * uniforms.rect.zw;
    float2 ndc = float2(
        pixel.x / max(uniforms.viewport.x, 1.0) * 2.0 - 1.0,
        1.0 - pixel.y / max(uniforms.viewport.y, 1.0) * 2.0
    );

    VertexOut out;
    out.position = float4(ndc, 0.0, 1.0);
    out.tex_coord = tex_coords[vertex_id];
    return out;
}

fragment float4 kuroko_overlay_fragment(
    VertexOut in [[stage_in]],
    texture2d<float, access::sample> overlay_texture [[texture(0)]],
    sampler overlay_sampler [[sampler(0)]]) {
    return overlay_texture.sample(overlay_sampler, in.tex_coord);
}
"#;

#[derive(Debug, Clone, Copy)]
struct PixelBufferMapping {
    format: ImportedVideoFormat,
    full_range: bool,
    planes: [PlaneMapping; 2],
}

impl PixelBufferMapping {
    fn from_core_video_format(pixel_format: u32) -> Option<Self> {
        match pixel_format {
            CV_PIXEL_FORMAT_420_YP_CB_CR10_BI_PLANAR_VIDEO_RANGE => Some(Self {
                format: ImportedVideoFormat::P010,
                full_range: false,
                planes: [
                    PlaneMapping {
                        index: 0,
                        pixel_format: MTLPixelFormat::R16Unorm,
                        name: "R16Unorm",
                    },
                    PlaneMapping {
                        index: 1,
                        pixel_format: MTLPixelFormat::RG16Unorm,
                        name: "RG16Unorm",
                    },
                ],
            }),
            CV_PIXEL_FORMAT_420_YP_CB_CR10_BI_PLANAR_FULL_RANGE => Some(Self {
                format: ImportedVideoFormat::P010,
                full_range: true,
                planes: [
                    PlaneMapping {
                        index: 0,
                        pixel_format: MTLPixelFormat::R16Unorm,
                        name: "R16Unorm",
                    },
                    PlaneMapping {
                        index: 1,
                        pixel_format: MTLPixelFormat::RG16Unorm,
                        name: "RG16Unorm",
                    },
                ],
            }),
            CV_PIXEL_FORMAT_420_YP_CB_CR8_BI_PLANAR_VIDEO_RANGE => Some(Self {
                format: ImportedVideoFormat::Nv12,
                full_range: false,
                planes: [
                    PlaneMapping {
                        index: 0,
                        pixel_format: MTLPixelFormat::R8Unorm,
                        name: "R8Unorm",
                    },
                    PlaneMapping {
                        index: 1,
                        pixel_format: MTLPixelFormat::RG8Unorm,
                        name: "RG8Unorm",
                    },
                ],
            }),
            CV_PIXEL_FORMAT_420_YP_CB_CR8_BI_PLANAR_FULL_RANGE => Some(Self {
                format: ImportedVideoFormat::Nv12,
                full_range: true,
                planes: [
                    PlaneMapping {
                        index: 0,
                        pixel_format: MTLPixelFormat::R8Unorm,
                        name: "R8Unorm",
                    },
                    PlaneMapping {
                        index: 1,
                        pixel_format: MTLPixelFormat::RG8Unorm,
                        name: "RG8Unorm",
                    },
                ],
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PlaneMapping {
    index: usize,
    pixel_format: MTLPixelFormat,
    name: &'static str,
}

fn create_plane_texture(
    cache: &CVMetalTextureCache,
    image_buffer: &CVImageBuffer,
    pixel_format: MTLPixelFormat,
    width: usize,
    height: usize,
    plane_index: usize,
) -> Result<CFRetained<CVMetalTexture>> {
    let mut raw_texture: *mut CVMetalTexture = std::ptr::null_mut();
    let status = unsafe {
        CVMetalTextureCache::create_texture_from_image(
            None,
            cache,
            image_buffer,
            None,
            pixel_format,
            width,
            height,
            plane_index,
            NonNull::new(&mut raw_texture as *mut *mut CVMetalTexture)
                .expect("stack texture pointer is non-null"),
        )
    };
    if status != kCVReturnSuccess {
        return Err(PlayerError::Renderer(format!(
            "CVMetalTextureCacheCreateTextureFromImage failed for plane {plane_index}: {status}"
        )));
    }
    let raw_texture = NonNull::new(raw_texture).ok_or_else(|| {
        PlayerError::Renderer(format!(
            "CVMetalTextureCacheCreateTextureFromImage returned null for plane {plane_index}"
        ))
    })?;
    Ok(unsafe { CFRetained::from_raw(raw_texture) })
}

#[cfg(test)]
mod tests {
    use super::VIDEO_SHADER_SOURCE;

    #[test]
    fn video_shader_has_bit_depth_aware_range_expansion() {
        assert!(VIDEO_SHADER_SOURCE.contains("uniforms.is_p010"));
        assert!(VIDEO_SHADER_SOURCE.contains("64.0 / 1023.0"));
        assert!(VIDEO_SHADER_SOURCE.contains("512.0 / 1023.0"));
        assert!(VIDEO_SHADER_SOURCE.contains("1023.0 / 876.0"));
        assert!(VIDEO_SHADER_SOURCE.contains("1023.0 / 896.0"));
        assert!(VIDEO_SHADER_SOURCE.contains("16.0 / 255.0"));
        assert!(VIDEO_SHADER_SOURCE.contains("128.0 / 255.0"));
        assert!(VIDEO_SHADER_SOURCE.contains("255.0 / 219.0"));
        assert!(VIDEO_SHADER_SOURCE.contains("255.0 / 224.0"));
    }

    #[test]
    fn video_shader_keeps_source_and_target_transfer_separate() {
        assert!(VIDEO_SHADER_SOURCE.contains("source_transfer"));
        assert!(VIDEO_SHADER_SOURCE.contains("target_transfer"));
    }
}
