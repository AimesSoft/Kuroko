use std::time::Duration;

use crate::danmaku::{DanmakuLayoutBox, DanmakuLayoutConfig, DanmakuTimeline};
use crate::subtitle::{SubtitleBitmapPlane, SubtitleFrame, SubtitleTimeline};
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
    subtitles: Option<SubtitleTimeline>,
    danmaku: Option<DanmakuTimeline>,
    shaper: TextShaper,
    danmaku_config: DanmakuLayoutConfig,
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
        self.subtitles = Some(subtitles);
        self
    }

    pub fn with_danmaku(mut self, danmaku: DanmakuTimeline) -> Self {
        self.danmaku = Some(danmaku);
        self
    }

    pub fn set_danmaku_config(&mut self, config: DanmakuLayoutConfig) {
        self.danmaku_config = config;
    }

    pub fn render(&self, pts: Duration, viewport: OverlayViewport) -> OverlayFrame {
        let subtitle_planes = self
            .subtitles
            .as_ref()
            .map(|timeline| subtitle_frame(timeline, pts, viewport).planes)
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

fn subtitle_frame(
    timeline: &SubtitleTimeline,
    pts: Duration,
    viewport: OverlayViewport,
) -> SubtitleFrame {
    timeline.render_debug_frame(pts, viewport.width, viewport.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::danmaku::{DanmakuItem, DanmakuMode};
    use crate::subtitle::{SubtitleCue, SubtitleTimeline};

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

        let frame = OverlayTimeline::default()
            .with_subtitles(subtitles)
            .with_danmaku(danmaku)
            .render(Duration::from_secs(2), OverlayViewport::new(640, 360));

        assert!(!frame.is_empty());
        assert_eq!(frame.subtitle_planes.len(), 1);
        assert_eq!(frame.danmaku_boxes.len(), 1);
        assert_eq!(frame.danmaku_boxes[0].item_id, 7);
    }
}
