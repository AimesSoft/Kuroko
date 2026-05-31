use crate::core::{PlatformSurface, PlayerError, RendererBackend, Result, WgpuSurfaceHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WgpuRendererStats {
    pub surface_width: u32,
    pub surface_height: u32,
    pub rendered_frames: u64,
    pub attached: bool,
}

pub struct WgpuRenderer {
    surface: Option<WgpuSurfaceHandle>,
    stats: WgpuRendererStats,
}

impl WgpuRenderer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            surface: None,
            stats: WgpuRendererStats::default(),
        })
    }

    pub fn surface(&self) -> Option<WgpuSurfaceHandle> {
        self.surface
    }

    pub fn stats(&self) -> WgpuRendererStats {
        self.stats
    }
}

impl RendererBackend for WgpuRenderer {
    fn attach_surface(&mut self, surface: PlatformSurface) -> Result<()> {
        let PlatformSurface::Wgpu(surface) = surface else {
            return Err(PlayerError::Renderer(
                "non-wgpu surface cannot be attached to WgpuRenderer".to_string(),
            ));
        };
        self.stats.surface_width = surface.width;
        self.stats.surface_height = surface.height;
        self.stats.attached = true;
        self.surface = Some(surface);
        Ok(())
    }

    fn detach_surface(&mut self) -> Result<()> {
        self.surface = None;
        self.stats.attached = false;
        Ok(())
    }

    fn resize_surface(&mut self, width: u32, height: u32, _scale: f64) -> Result<()> {
        let Some(mut surface) = self.surface else {
            return Err(PlayerError::Renderer(
                "no wgpu surface attached".to_string(),
            ));
        };
        surface.width = width;
        surface.height = height;
        self.surface = Some(surface);
        self.stats.surface_width = width;
        self.stats.surface_height = height;
        Ok(())
    }

    fn render_test_frame(&mut self, _time_seconds: f64) -> Result<()> {
        if self.surface.is_none() {
            return Err(PlayerError::Renderer(
                "no wgpu surface attached".to_string(),
            ));
        }
        self.stats.rendered_frames += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::WgpuSurfaceKind;

    #[test]
    fn wgpu_renderer_tracks_surface_lifecycle() {
        let mut renderer = WgpuRenderer::new().unwrap();
        let surface = PlatformSurface::Wgpu(WgpuSurfaceHandle::new(
            WgpuSurfaceKind::MacOsCaMetalLayer,
            42,
            0,
            640,
            360,
            2.0,
        ));

        renderer.attach_surface(surface).unwrap();
        renderer.render_test_frame(0.0).unwrap();
        renderer.resize_surface(1280, 720, 2.0).unwrap();

        let stats = renderer.stats();
        assert!(stats.attached);
        assert_eq!(stats.surface_width, 1280);
        assert_eq!(stats.surface_height, 720);
        assert_eq!(stats.rendered_frames, 1);

        renderer.detach_surface().unwrap();
        assert!(!renderer.stats().attached);
    }

    #[test]
    fn wgpu_renderer_rejects_metal_surface() {
        let mut renderer = WgpuRenderer::new().unwrap();
        let result = renderer.attach_surface(PlatformSurface::Metal(
            crate::core::MetalSurfaceHandle::new(42, 640, 360, 2.0),
        ));

        assert!(matches!(result, Err(PlayerError::Renderer(_))));
    }
}
