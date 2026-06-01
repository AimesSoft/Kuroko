#[cfg(feature = "libass")]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::danmaku::{DanmakuLayoutBox, DanmakuLayoutConfig, DanmakuTimeline};
#[cfg(feature = "libass")]
use crate::subtitle::{
    LibassRenderConfig, LibassSubtitleRenderer, SubtitleError, SubtitleRenderOutput,
    SubtitleRenderRequest, SubtitleRenderer,
};
use crate::subtitle::{
    Result as SubtitleResult, SubtitleBitmapPlane, SubtitleFrame, SubtitleRendererCore,
    SubtitleTimeline, SubtitleViewport,
};
use crate::text::TextShaper;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayViewport {
    pub width: u32,
    pub height: u32,
}

impl OverlayViewport {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayFrame {
    pub pts: Duration,
    pub viewport: OverlayViewport,
    pub subtitle_planes: Vec<SubtitleBitmapPlane>,
    pub danmaku_boxes: Vec<DanmakuLayoutBox>,
}

impl OverlayFrame {
    pub fn is_empty(&self) -> bool {
        self.subtitle_planes.is_empty() && self.danmaku_boxes.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct OverlayTimeline {
    subtitles: Option<OverlaySubtitleRenderer>,
    danmaku: Option<DanmakuTimeline>,
    shaper: TextShaper,
    danmaku_config: DanmakuLayoutConfig,
}

#[derive(Debug, Clone)]
enum OverlaySubtitleRenderer {
    DebugTimeline(SubtitleRendererCore),
    #[cfg(feature = "libass")]
    Libass(Arc<Mutex<LibassSubtitleRenderer>>),
}

impl OverlaySubtitleRenderer {
    fn render(
        &mut self,
        pts: Duration,
        viewport: OverlayViewport,
    ) -> SubtitleResult<SubtitleFrame> {
        match self {
            Self::DebugTimeline(renderer) => Ok(renderer
                .render(pts, SubtitleViewport::new(viewport.width, viewport.height))
                .frame),
            #[cfg(feature = "libass")]
            Self::Libass(renderer) => {
                let mut renderer = renderer
                    .lock()
                    .map_err(|_| SubtitleError::Libass("renderer lock poisoned".to_string()))?;
                renderer
                    .render(SubtitleRenderRequest::new(
                        pts,
                        viewport.width,
                        viewport.height,
                    ))
                    .map(SubtitleRenderOutput::into_rgba_frame)
            }
        }
    }
}

impl OverlayTimeline {
    pub fn new(shaper: TextShaper) -> Self {
        Self {
            subtitles: None,
            danmaku: None,
            shaper,
            danmaku_config: DanmakuLayoutConfig::default(),
        }
    }

    pub fn with_subtitles(mut self, subtitles: SubtitleTimeline) -> Self {
        self.subtitles = Some(OverlaySubtitleRenderer::DebugTimeline(
            SubtitleRendererCore::new_debug(subtitles),
        ));
        self
    }

    #[cfg(feature = "libass")]
    pub fn with_ass_subtitles(
        mut self,
        script: impl AsRef<[u8]>,
        config: LibassRenderConfig,
    ) -> SubtitleResult<Self> {
        self.subtitles = Some(OverlaySubtitleRenderer::Libass(Arc::new(Mutex::new(
            LibassSubtitleRenderer::from_ass_script(script, config)?,
        ))));
        Ok(self)
    }

    #[cfg(feature = "libass")]
    pub fn with_libass_renderer(mut self, renderer: LibassSubtitleRenderer) -> Self {
        self.subtitles = Some(OverlaySubtitleRenderer::Libass(Arc::new(Mutex::new(
            renderer,
        ))));
        self
    }

    pub fn with_danmaku(mut self, danmaku: DanmakuTimeline) -> Self {
        self.danmaku = Some(danmaku);
        self
    }

    pub fn set_danmaku_config(&mut self, config: DanmakuLayoutConfig) {
        self.danmaku_config = config;
    }

    pub fn render(&mut self, pts: Duration, viewport: OverlayViewport) -> OverlayFrame {
        let subtitle_planes = self
            .subtitles
            .as_mut()
            .and_then(|renderer| match renderer.render(pts, viewport) {
                Ok(frame) => Some(frame.planes),
                Err(error) => {
                    eprintln!("Kuroko overlay subtitle render failed: {error}");
                    None
                }
            })
            .unwrap_or_default();

        let danmaku_boxes = self
            .danmaku
            .as_ref()
            .map(|timeline| {
                let mut config = self.danmaku_config;
                config.viewport_width = viewport.width as f32;
                config.viewport_height = viewport.height as f32;
                timeline.layout(pts, config, &self.shaper)
            })
            .unwrap_or_default();

        OverlayFrame {
            pts,
            viewport,
            subtitle_planes,
            danmaku_boxes,
        }
    }
}

impl Default for OverlayTimeline {
    fn default() -> Self {
        Self::new(TextShaper::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::danmaku::{DanmakuItem, DanmakuMode};
    use crate::subtitle::{SubtitleCue, SubtitleTimeline};

    #[cfg(feature = "libass")]
    const SIMPLE_ASS_SCRIPT: &str = r#"[Script Info]
ScriptType: v4.00+
PlayResX: 640
PlayResY: 360

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,32,&H00FFFFFF,&H000000FF,&H80000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,2,20,20,24,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,Overlay libass
"#;

    #[test]
    fn overlay_timeline_combines_subtitles_and_danmaku() {
        let subtitles = SubtitleTimeline::new(vec![SubtitleCue {
            start: Duration::from_secs(1),
            end: Duration::from_secs(3),
            text: "hello".to_string(),
        }]);
        let mut danmaku = DanmakuTimeline::default();
        danmaku.push(DanmakuItem {
            id: 7,
            pts: Duration::from_secs(2),
            text: "native comment".to_string(),
            mode: DanmakuMode::Scroll,
            font_size: 24.0,
            color_rgba: [1.0, 1.0, 1.0, 1.0],
        });

        let mut timeline = OverlayTimeline::default()
            .with_subtitles(subtitles)
            .with_danmaku(danmaku);
        let frame = timeline.render(Duration::from_secs(2), OverlayViewport::new(640, 360));

        assert!(!frame.is_empty());
        assert_eq!(frame.subtitle_planes.len(), 1);
        assert_eq!(frame.danmaku_boxes.len(), 1);
        assert_eq!(frame.danmaku_boxes[0].item_id, 7);
    }

    #[cfg(feature = "libass")]
    #[test]
    fn overlay_timeline_renders_libass_subtitles() {
        let mut timeline = OverlayTimeline::default()
            .with_ass_subtitles(SIMPLE_ASS_SCRIPT, LibassRenderConfig::default())
            .unwrap();

        let frame = timeline.render(Duration::from_millis(500), OverlayViewport::new(640, 360));

        assert_eq!(frame.pts, Duration::from_millis(500));
        assert_eq!(frame.viewport, OverlayViewport::new(640, 360));
        assert!(!frame.subtitle_planes.is_empty());
        assert!(frame.danmaku_boxes.is_empty());
        assert!(
            frame.subtitle_planes.iter().all(|plane| {
                plane.rgba.len() == plane.width as usize * plane.height as usize * 4
            })
        );
    }
}
