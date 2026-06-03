#[cfg(feature = "danmaku-next2")]
use std::sync::mpsc;
#[cfg(any(feature = "libass", feature = "danmaku-next2"))]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::danmaku::{DanmakuLayoutBox, DanmakuLayoutConfig, DanmakuTimeline};
#[cfg(feature = "danmaku-next2")]
use crate::danmaku_next2::{
    Next2TimelineConfig,
    engine::{EngineCommand, RenderFrameInput, create_engine, lookup_engine, remove_engine},
    layout,
};
#[cfg(feature = "libass")]
use crate::subtitle::{
    LibassRenderConfig, LibassSubtitleRenderer, SubtitleError, SubtitleRenderRequest,
    SubtitleRenderer,
};
use crate::subtitle::{
    Result as SubtitleResult, SubtitleAlphaBitmap, SubtitleBitmapPlane, SubtitleFrameChange,
    SubtitleRenderOutput, SubtitleRendererCore, SubtitleTimeline, SubtitleViewport,
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
    /// Video-source coordinate space. Subtitle bitmaps are authored in this space
    /// and are mapped through the video presentation rect by the renderer.
    pub viewport: OverlayViewport,
    /// Full output surface coordinate space. Danmaku is authored in this space so
    /// it can cover letterbox/pillarbox areas while video keeps its own aspect.
    pub surface_viewport: OverlayViewport,
    pub subtitle_planes: Vec<SubtitleBitmapPlane>,
    pub subtitle_alpha_planes: Vec<SubtitleAlphaBitmap>,
    pub danmaku_planes: Vec<SubtitleBitmapPlane>,
    pub subtitle_changed: bool,
    pub danmaku_boxes: Vec<DanmakuLayoutBox>,
}

impl OverlayFrame {
    pub fn is_empty(&self) -> bool {
        self.subtitle_planes.is_empty()
            && self.subtitle_alpha_planes.is_empty()
            && self.danmaku_planes.is_empty()
            && self.danmaku_boxes.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct OverlayTimeline {
    subtitles: Option<OverlaySubtitleRenderer>,
    danmaku: Option<DanmakuTimeline>,
    #[cfg(feature = "libass")]
    danmaku_renderer: Option<Arc<Mutex<LibassSubtitleRenderer>>>,
    #[cfg(feature = "danmaku-next2")]
    next2_danmaku_renderer: Option<Arc<Mutex<Next2DanmakuOverlayRenderer>>>,
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
    ) -> SubtitleResult<(SubtitleRenderOutput, bool)> {
        match self {
            Self::DebugTimeline(renderer) => {
                let result =
                    renderer.render(pts, SubtitleViewport::new(viewport.width, viewport.height));
                Ok((
                    SubtitleRenderOutput::Rgba(result.frame),
                    result.change == SubtitleFrameChange::Changed,
                ))
            }
            #[cfg(feature = "libass")]
            Self::Libass(renderer) => {
                let mut renderer = renderer
                    .lock()
                    .map_err(|_| SubtitleError::Libass("renderer lock poisoned".to_string()))?;
                let output = renderer.render(SubtitleRenderRequest::new(
                    pts,
                    viewport.width,
                    viewport.height,
                ))?;
                let changed = match &output {
                    SubtitleRenderOutput::Rgba(_) => true,
                    SubtitleRenderOutput::Alpha(bitmaps) => bitmaps.changed,
                };
                Ok((output, changed))
            }
        }
    }
}

impl OverlayTimeline {
    pub fn new(shaper: TextShaper) -> Self {
        Self {
            subtitles: None,
            danmaku: None,
            #[cfg(feature = "libass")]
            danmaku_renderer: None,
            #[cfg(feature = "danmaku-next2")]
            next2_danmaku_renderer: None,
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
        #[cfg(feature = "danmaku-next2")]
        match Next2DanmakuOverlayRenderer::new(danmaku.clone(), self.danmaku_config) {
            Ok(renderer) => {
                self.next2_danmaku_renderer = Some(Arc::new(Mutex::new(renderer)));
            }
            Err(error) => {
                eprintln!("Kuroko overlay Next2 danmaku setup failed: {error}");
            }
        }
        self.danmaku = Some(danmaku);
        self
    }

    #[cfg(feature = "danmaku-next2")]
    pub fn with_next2_danmaku(
        mut self,
        danmaku: DanmakuTimeline,
    ) -> std::result::Result<Self, String> {
        self.next2_danmaku_renderer = Some(Arc::new(Mutex::new(Next2DanmakuOverlayRenderer::new(
            danmaku.clone(),
            self.danmaku_config,
        )?)));
        self.danmaku = Some(danmaku);
        Ok(self)
    }

    #[cfg(feature = "libass")]
    pub fn with_ass_danmaku(
        mut self,
        danmaku: DanmakuTimeline,
        config: LibassRenderConfig,
    ) -> SubtitleResult<Self> {
        let script = danmaku.to_ass_script(self.danmaku_config, &self.shaper);
        self.danmaku_renderer = Some(Arc::new(Mutex::new(
            LibassSubtitleRenderer::from_ass_script(script, config)?,
        )));
        self.danmaku = Some(danmaku);
        Ok(self)
    }

    pub fn set_danmaku_config(&mut self, config: DanmakuLayoutConfig) {
        self.danmaku_config = config;
    }

    pub fn render(&mut self, pts: Duration, viewport: OverlayViewport) -> OverlayFrame {
        self.render_for_surface(pts, viewport, viewport)
    }

    pub fn render_for_surface(
        &mut self,
        pts: Duration,
        video_viewport: OverlayViewport,
        surface_viewport: OverlayViewport,
    ) -> OverlayFrame {
        let subtitle_result = match self
            .subtitles
            .as_mut()
            .map(|renderer| renderer.render(pts, video_viewport))
        {
            Some(Ok((SubtitleRenderOutput::Rgba(frame), changed))) => {
                (frame.planes, Vec::new(), changed)
            }
            Some(Ok((SubtitleRenderOutput::Alpha(bitmaps), changed))) => {
                (Vec::new(), bitmaps.parts, changed)
            }
            Some(Err(error)) => {
                eprintln!("Kuroko overlay subtitle render failed: {error}");
                (Vec::new(), Vec::new(), true)
            }
            None => (Vec::new(), Vec::new(), false),
        };
        #[cfg(feature = "libass")]
        let (mut subtitle_planes, mut subtitle_alpha_planes, mut subtitle_changed) =
            subtitle_result;
        #[cfg(all(not(feature = "libass"), feature = "danmaku-next2"))]
        let (subtitle_planes, subtitle_alpha_planes, mut subtitle_changed) = subtitle_result;
        #[cfg(not(any(feature = "libass", feature = "danmaku-next2")))]
        let (subtitle_planes, subtitle_alpha_planes, subtitle_changed) = subtitle_result;

        #[cfg(feature = "libass")]
        if let Some(renderer) = &self.danmaku_renderer {
            match render_libass_overlay_renderer(renderer, pts, surface_viewport) {
                Ok((SubtitleRenderOutput::Rgba(frame), changed)) => {
                    subtitle_planes.extend(frame.planes);
                    subtitle_changed |= changed;
                }
                Ok((SubtitleRenderOutput::Alpha(bitmaps), changed)) => {
                    subtitle_alpha_planes.extend(bitmaps.parts);
                    subtitle_changed |= changed;
                }
                Err(error) => {
                    eprintln!("Kuroko overlay danmaku render failed: {error}");
                    subtitle_changed = true;
                }
            }
        }

        #[cfg(feature = "danmaku-next2")]
        let mut danmaku_planes = Vec::new();
        #[cfg(not(feature = "danmaku-next2"))]
        let danmaku_planes = Vec::new();

        #[cfg(feature = "danmaku-next2")]
        let mut next2_danmaku_boxes = None;

        #[cfg(feature = "danmaku-next2")]
        if let Some(renderer) = &self.next2_danmaku_renderer {
            match renderer
                .lock()
                .map_err(|_| "next2 danmaku renderer lock poisoned".to_string())
                .and_then(|mut renderer| renderer.render(pts, surface_viewport))
            {
                Ok(rendered) => {
                    next2_danmaku_boxes = Some(rendered.boxes);
                    if let Some(plane) = rendered.plane {
                        danmaku_planes.push(plane);
                    }
                    if !danmaku_planes.is_empty()
                        || !next2_danmaku_boxes.as_ref().is_some_and(Vec::is_empty)
                    {
                        subtitle_changed = true;
                    }
                }
                Err(error) => {
                    eprintln!("Kuroko overlay Next2 danmaku render failed: {error}");
                    subtitle_changed = true;
                }
            }
        }

        #[cfg(feature = "danmaku-next2")]
        let danmaku_boxes =
            next2_danmaku_boxes.unwrap_or_else(|| self.legacy_danmaku_boxes(pts, surface_viewport));
        #[cfg(not(feature = "danmaku-next2"))]
        let danmaku_boxes = self.legacy_danmaku_boxes(pts, surface_viewport);

        OverlayFrame {
            pts,
            viewport: video_viewport,
            surface_viewport,
            subtitle_planes,
            subtitle_alpha_planes,
            danmaku_planes,
            subtitle_changed,
            danmaku_boxes,
        }
    }

    fn legacy_danmaku_boxes(
        &self,
        pts: Duration,
        surface_viewport: OverlayViewport,
    ) -> Vec<DanmakuLayoutBox> {
        self.danmaku
            .as_ref()
            .map(|timeline| {
                let mut config = self.danmaku_config;
                config.viewport_width = surface_viewport.width as f32;
                config.viewport_height = surface_viewport.height as f32;
                timeline.layout(pts, config, &self.shaper)
            })
            .unwrap_or_default()
    }
}

#[cfg(feature = "libass")]
fn render_libass_overlay_renderer(
    renderer: &Arc<Mutex<LibassSubtitleRenderer>>,
    pts: Duration,
    viewport: OverlayViewport,
) -> SubtitleResult<(SubtitleRenderOutput, bool)> {
    let mut renderer = renderer
        .lock()
        .map_err(|_| SubtitleError::Libass("renderer lock poisoned".to_string()))?;
    let output = renderer.render(SubtitleRenderRequest::new(
        pts,
        viewport.width,
        viewport.height,
    ))?;
    let changed = match &output {
        SubtitleRenderOutput::Rgba(_) => true,
        SubtitleRenderOutput::Alpha(bitmaps) => bitmaps.changed,
    };
    Ok((output, changed))
}

#[cfg(feature = "danmaku-next2")]
struct Next2DanmakuOverlayRenderer {
    timeline: DanmakuTimeline,
    layout_config: DanmakuLayoutConfig,
    prepared: Option<layout::RustNext2PreparedLayout>,
    viewport: Option<OverlayViewport>,
    pending_frame: bool,
    pending_reply: Option<mpsc::Receiver<bool>>,
    last_submitted_frame_json: Option<String>,
    last_plane: Option<SubtitleBitmapPlane>,
    handle: u64,
}

#[cfg(feature = "danmaku-next2")]
struct Next2DanmakuRenderedFrame {
    plane: Option<SubtitleBitmapPlane>,
    boxes: Vec<DanmakuLayoutBox>,
}

#[cfg(feature = "danmaku-next2")]
impl std::fmt::Debug for Next2DanmakuOverlayRenderer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Next2DanmakuOverlayRenderer")
            .field("items", &self.timeline.items().len())
            .field("viewport", &self.viewport)
            .field("handle", &self.handle)
            .finish()
    }
}

#[cfg(feature = "danmaku-next2")]
impl Next2DanmakuOverlayRenderer {
    fn new(
        timeline: DanmakuTimeline,
        layout_config: DanmakuLayoutConfig,
    ) -> std::result::Result<Self, String> {
        Ok(Self {
            timeline,
            layout_config,
            prepared: None,
            viewport: None,
            pending_frame: false,
            pending_reply: None,
            last_submitted_frame_json: None,
            last_plane: None,
            handle: create_engine(2, 2)?,
        })
    }

    fn render(
        &mut self,
        pts: Duration,
        viewport: OverlayViewport,
    ) -> std::result::Result<Next2DanmakuRenderedFrame, String> {
        self.ensure_viewport(viewport)?;
        let Some(prepared) = &self.prepared else {
            return Ok(Next2DanmakuRenderedFrame {
                plane: None,
                boxes: Vec::new(),
            });
        };
        let frame = layout::next2_layout_frame_ref(prepared, pts.as_secs_f64());
        let boxes = next2_frame_boxes(&frame, self.layout_config_font_size());
        if frame.items.is_empty() {
            self.pending_frame = false;
            self.pending_reply = None;
            self.last_submitted_frame_json = None;
            self.last_plane = None;
            self.clear_latest_frame()?;
            return Ok(Next2DanmakuRenderedFrame { plane: None, boxes });
        }

        self.poll_pending_reply();
        self.take_latest_frame()?;
        let frame_json = next2_frame_json(&frame);
        if !self.pending_frame && self.last_submitted_frame_json.as_ref() != Some(&frame_json) {
            self.submit_frame(frame_json)?;
        }

        Ok(Next2DanmakuRenderedFrame {
            plane: self.last_plane.clone(),
            boxes,
        })
    }

    fn submit_frame(&mut self, frame_json: String) -> std::result::Result<(), String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let input = RenderFrameInput {
            frame_json: frame_json.clone(),
            font_size: self.layout_config_font_size(),
            outline_width: 1.0,
            shadow_style: 2,
            opacity: 1.0,
            custom_font_family: String::new(),
            custom_font_file_path: String::new(),
        };
        let entry = lookup_engine(self.handle)
            .ok_or_else(|| "next2 danmaku engine handle missing".to_string())?;
        entry
            .cmd_tx
            .send(EngineCommand::SetFrame {
                input,
                reply: reply_tx,
            })
            .map_err(|error| format!("next2 danmaku set frame send failed: {error}"))?;
        self.pending_frame = true;
        self.pending_reply = Some(reply_rx);
        self.last_submitted_frame_json = Some(frame_json);
        Ok(())
    }

    fn poll_pending_reply(&mut self) {
        let Some(reply) = self.pending_reply.as_ref() else {
            return;
        };
        match reply.try_recv() {
            Ok(true) => {
                self.pending_reply = None;
            }
            Ok(false) | Err(mpsc::TryRecvError::Disconnected) => {
                self.pending_frame = false;
                self.pending_reply = None;
                self.last_submitted_frame_json = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn take_latest_frame(&mut self) -> std::result::Result<(), String> {
        let entry = lookup_engine(self.handle)
            .ok_or_else(|| "next2 danmaku engine handle missing".to_string())?;
        let mut frame = entry
            .latest_frame
            .lock()
            .map_err(|_| "next2 danmaku frame lock poisoned".to_string())?;
        let Some(frame) = frame.take() else {
            return Ok(());
        };
        if frame.width
            == self
                .viewport
                .map(|viewport| viewport.width)
                .unwrap_or(frame.width)
            && frame.height
                == self
                    .viewport
                    .map(|viewport| viewport.height)
                    .unwrap_or(frame.height)
        {
            self.last_plane = cropped_bgra_to_rgba_plane(&frame);
        }
        self.pending_frame = false;
        self.pending_reply = None;
        Ok(())
    }

    fn ensure_viewport(&mut self, viewport: OverlayViewport) -> std::result::Result<(), String> {
        if self.viewport == Some(viewport) {
            return Ok(());
        }
        let entry = lookup_engine(self.handle)
            .ok_or_else(|| "next2 danmaku engine handle missing".to_string())?;
        entry
            .cmd_tx
            .send(EngineCommand::Resize {
                width: viewport.width,
                height: viewport.height,
            })
            .map_err(|error| format!("next2 danmaku resize send failed: {error}"))?;
        let config = Next2TimelineConfig {
            width: f64::from(viewport.width),
            height: f64::from(viewport.height),
            font_size: f64::from(self.layout_config_font_size()),
            display_area: 1.0,
            scroll_duration: self.layout_config.duration,
            allow_stacking: false,
            merge_danmaku: false,
        };
        self.prepared = Some(crate::danmaku_next2::prepare_timeline_layout(
            &self.timeline,
            config,
        )?);
        self.viewport = Some(viewport);
        self.pending_frame = false;
        self.pending_reply = None;
        self.last_submitted_frame_json = None;
        self.last_plane = None;
        self.clear_latest_frame()?;
        Ok(())
    }

    fn clear_latest_frame(&self) -> std::result::Result<(), String> {
        let entry = lookup_engine(self.handle)
            .ok_or_else(|| "next2 danmaku engine handle missing".to_string())?;
        let mut frame = entry
            .latest_frame
            .lock()
            .map_err(|_| "next2 danmaku frame lock poisoned".to_string())?;
        *frame = None;
        Ok(())
    }

    fn layout_config_font_size(&self) -> f32 {
        25.0
    }
}

#[cfg(feature = "danmaku-next2")]
impl Drop for Next2DanmakuOverlayRenderer {
    fn drop(&mut self) {
        if let Some(entry) = remove_engine(self.handle) {
            let _ = entry.cmd_tx.send(EngineCommand::Stop);
        }
    }
}

#[cfg(feature = "danmaku-next2")]
fn next2_frame_json(frame: &layout::RustNext2FrameLayout) -> String {
    let mut json = String::with_capacity(frame.items.len().saturating_mul(96).saturating_add(16));
    json.push_str("{\"items\":[");
    for (index, item) in frame.items.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str("{\"text\":\"");
        push_json_string(&mut json, &item.text);
        json.push_str("\",");
        if let Some(count_text) = &item.count_text {
            json.push_str("\"count_text\":\"");
            push_json_string(&mut json, count_text);
            json.push_str("\",");
        }
        json.push_str(&format!(
            "\"x\":{:.3},\"y\":{:.3},\"color_argb\":{},\"font_size_multiplier\":{:.3}",
            item.x, item.y, item.color_argb, item.font_size_multiplier
        ));
        json.push('}');
    }
    json.push_str("]}");
    json
}

#[cfg(feature = "danmaku-next2")]
fn next2_frame_boxes(
    frame: &layout::RustNext2FrameLayout,
    base_font_size: f32,
) -> Vec<DanmakuLayoutBox> {
    frame
        .items
        .iter()
        .map(|item| DanmakuLayoutBox {
            item_id: item.id,
            text: next2_box_text(item),
            x: item.x as f32,
            y: item.y as f32,
            width: item.width.max(0.0) as f32,
            height: item.height.max(0.0) as f32,
            font_size: (f64::from(base_font_size) * item.font_size_multiplier).max(1.0) as f32,
            color_rgba: color_from_next2_argb(item.color_argb),
        })
        .collect()
}

#[cfg(feature = "danmaku-next2")]
fn next2_box_text(item: &layout::RustNext2FrameItem) -> String {
    let Some(count_text) = &item.count_text else {
        return item.text.clone();
    };
    if count_text.is_empty() {
        item.text.clone()
    } else {
        format!("{} {}", item.text, count_text)
    }
}

#[cfg(feature = "danmaku-next2")]
fn color_from_next2_argb(color_argb: i32) -> [f32; 4] {
    let value = color_argb as u32;
    [
        ((value >> 16) & 0xff) as f32 / 255.0,
        ((value >> 8) & 0xff) as f32 / 255.0,
        (value & 0xff) as f32 / 255.0,
        ((value >> 24) & 0xff) as f32 / 255.0,
    ]
}

#[cfg(feature = "danmaku-next2")]
fn push_json_string(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
}

#[cfg(feature = "danmaku-next2")]
fn cropped_bgra_to_rgba_plane(
    frame: &crate::danmaku_next2::engine::Next2ReadbackFrame,
) -> Option<SubtitleBitmapPlane> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    if width == 0
        || height == 0
        || frame.pixels.len() < width.saturating_mul(height).saturating_mul(4)
    {
        return None;
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    for y in 0..height {
        for x in 0..width {
            let alpha = frame.pixels[(y * width + x) * 4 + 3];
            if alpha == 0 {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if min_x > max_x || min_y > max_y {
        return None;
    }

    let crop_width = max_x - min_x + 1;
    let crop_height = max_y - min_y + 1;
    let mut rgba = Vec::with_capacity(crop_width * crop_height * 4);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let offset = (y * width + x) * 4;
            let blue = frame.pixels[offset];
            let green = frame.pixels[offset + 1];
            let red = frame.pixels[offset + 2];
            let alpha = frame.pixels[offset + 3];
            let (red, green, blue) = unpremultiply_rgb(red, green, blue, alpha);
            rgba.extend_from_slice(&[red, green, blue, alpha]);
        }
    }

    Some(SubtitleBitmapPlane {
        x: min_x.min(i32::MAX as usize) as i32,
        y: min_y.min(i32::MAX as usize) as i32,
        width: crop_width.min(u32::MAX as usize) as u32,
        height: crop_height.min(u32::MAX as usize) as u32,
        rgba,
    })
}

#[cfg(feature = "danmaku-next2")]
fn unpremultiply_rgb(red: u8, green: u8, blue: u8, alpha: u8) -> (u8, u8, u8) {
    if alpha == 0 {
        return (0, 0, 0);
    }
    let alpha = u16::from(alpha);
    (
        ((u16::from(red) * 255 + alpha / 2) / alpha).min(255) as u8,
        ((u16::from(green) * 255 + alpha / 2) / alpha).min(255) as u8,
        ((u16::from(blue) * 255 + alpha / 2) / alpha).min(255) as u8,
    )
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
        assert!(frame.subtitle_alpha_planes.is_empty());
        assert!(frame.subtitle_changed);
        assert_eq!(frame.danmaku_boxes.len(), 1);
        assert_eq!(frame.danmaku_boxes[0].item_id, 7);
        assert_eq!(frame.danmaku_boxes[0].text, "native comment");
    }

    #[test]
    fn overlay_timeline_lays_out_danmaku_in_surface_space() {
        let mut danmaku = DanmakuTimeline::default();
        danmaku.push(DanmakuItem {
            id: 9,
            pts: Duration::from_secs(2),
            text: "fullscreen danmaku".to_string(),
            mode: DanmakuMode::Top,
            font_size: 24.0,
            color_rgba: [1.0, 1.0, 1.0, 1.0],
        });
        let mut timeline = OverlayTimeline::default().with_danmaku(danmaku);

        let frame = timeline.render_for_surface(
            Duration::from_secs(2),
            OverlayViewport::new(640, 360),
            OverlayViewport::new(1280, 720),
        );

        assert_eq!(frame.viewport, OverlayViewport::new(640, 360));
        assert_eq!(frame.surface_viewport, OverlayViewport::new(1280, 720));
        assert_eq!(frame.danmaku_boxes.len(), 1);
        assert!(frame.danmaku_boxes[0].x > 320.0);
        assert!(frame.danmaku_boxes[0].y < 720.0);
    }

    #[test]
    fn overlay_timeline_reports_unchanged_debug_subtitles() {
        let subtitles = SubtitleTimeline::new(vec![SubtitleCue {
            start: Duration::from_secs(1),
            end: Duration::from_secs(3),
            text: "hello".to_string(),
        }]);
        let mut timeline = OverlayTimeline::default().with_subtitles(subtitles);

        let first = timeline.render(Duration::from_secs(2), OverlayViewport::new(640, 360));
        let second = timeline.render(Duration::from_secs(2), OverlayViewport::new(640, 360));

        assert!(first.subtitle_changed);
        assert!(!second.subtitle_changed);
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
        assert!(frame.subtitle_planes.is_empty());
        assert!(!frame.subtitle_alpha_planes.is_empty());
        assert!(frame.subtitle_changed);
        assert!(frame.danmaku_boxes.is_empty());
        assert!(
            frame
                .subtitle_alpha_planes
                .iter()
                .all(SubtitleAlphaBitmap::is_valid)
        );
    }

    #[cfg(feature = "libass")]
    #[test]
    fn overlay_timeline_renders_ass_danmaku_into_alpha_planes() {
        let mut danmaku = DanmakuTimeline::default();
        danmaku.push(DanmakuItem {
            id: 8,
            pts: Duration::ZERO,
            text: "ass danmaku".to_string(),
            mode: DanmakuMode::Scroll,
            font_size: 32.0,
            color_rgba: [1.0, 1.0, 1.0, 1.0],
        });
        let mut timeline = OverlayTimeline::default()
            .with_ass_danmaku(danmaku, LibassRenderConfig::default())
            .unwrap();

        let frame = timeline.render(Duration::from_millis(200), OverlayViewport::new(640, 360));

        assert_eq!(frame.danmaku_boxes.len(), 1);
        assert_eq!(frame.danmaku_boxes[0].text, "ass danmaku");
        assert!(!frame.subtitle_alpha_planes.is_empty());
        assert!(frame.subtitle_changed);
    }

    #[cfg(feature = "danmaku-next2")]
    #[test]
    fn next2_danmaku_boxes_preserve_source_metadata() {
        let mut danmaku = DanmakuTimeline::default();
        danmaku.push(DanmakuItem {
            id: 42,
            pts: Duration::ZERO,
            text: "next2 box".to_string(),
            mode: DanmakuMode::Top,
            font_size: 50.0,
            color_rgba: [1.0, 0.0, 0.0, 1.0],
        });
        let mut timeline = OverlayTimeline::default()
            .with_next2_danmaku(danmaku)
            .unwrap();

        let frame = timeline.render_for_surface(
            Duration::from_millis(250),
            OverlayViewport::new(640, 360),
            OverlayViewport::new(1280, 720),
        );

        assert_eq!(frame.viewport, OverlayViewport::new(640, 360));
        assert_eq!(frame.surface_viewport, OverlayViewport::new(1280, 720));
        assert_eq!(frame.danmaku_boxes.len(), 1);
        assert_eq!(frame.danmaku_boxes[0].item_id, 42);
        assert_eq!(frame.danmaku_boxes[0].text, "next2 box");
        assert_eq!(frame.danmaku_boxes[0].font_size, 50.0);
        assert_eq!(frame.danmaku_boxes[0].color_rgba, [1.0, 0.0, 0.0, 1.0]);
        assert!(frame.danmaku_boxes[0].x > 320.0);
    }
}
