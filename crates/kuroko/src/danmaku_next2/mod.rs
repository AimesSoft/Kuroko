//! NipaPlay Next2/DFM+ danmaku engine port.
//!
//! This module intentionally keeps the imported engine isolated while Kuroko's
//! RendererCore integration is being built. The layout/track code is always
//! available; the MSDF/wgpu renderer is behind `danmaku-next2`.
#![allow(dead_code)]

use std::time::Duration;

use crate::danmaku::{DanmakuMode, DanmakuTimeline};

pub mod dfm_core;
pub mod layout;

#[cfg(feature = "danmaku-next2")]
pub mod engine;
#[cfg(feature = "danmaku-next2")]
mod present;

#[derive(Debug, Clone, PartialEq)]
pub struct Next2TimelineConfig {
    pub width: f64,
    pub height: f64,
    pub font_size: f64,
    pub display_area: f64,
    pub scroll_duration: Duration,
    pub allow_stacking: bool,
    pub merge_danmaku: bool,
}

impl Default for Next2TimelineConfig {
    fn default() -> Self {
        Self {
            width: 1920.0,
            height: 1080.0,
            font_size: 25.0,
            display_area: 1.0,
            scroll_duration: Duration::from_secs(9),
            allow_stacking: false,
            merge_danmaku: false,
        }
    }
}

pub fn prepare_timeline_layout(
    timeline: &DanmakuTimeline,
    config: Next2TimelineConfig,
) -> Result<layout::RustNext2PreparedLayout, String> {
    let items = timeline
        .items()
        .iter()
        .map(|item| layout::RustNext2DanmakuItem {
            id: item.id,
            time_seconds: item.pts.as_secs_f64(),
            text: item.text.clone(),
            type_code: next2_type_code(item.mode),
            color_argb: rgba_to_argb_i32(item.color_rgba),
            is_me: false,
            font_size_multiplier: font_size_multiplier(item.font_size, config.font_size),
        })
        .collect();

    layout::next2_prepare_layout(layout::RustNext2PrepareRequest {
        items,
        width: config.width,
        height: config.height,
        font_size: config.font_size,
        display_area: config.display_area,
        scroll_duration_seconds: config.scroll_duration.as_secs_f64(),
        allow_stacking: config.allow_stacking,
        merge_danmaku: config.merge_danmaku,
        custom_font_family: String::new(),
        custom_font_file_path: String::new(),
    })
}

pub fn layout_prepared_frame(
    prepared: layout::RustNext2PreparedLayout,
    pts: Duration,
) -> layout::RustNext2FrameLayout {
    layout::next2_layout_frame(layout::RustNext2FrameRequest {
        layout: prepared,
        current_time_seconds: pts.as_secs_f64(),
    })
}

fn next2_type_code(mode: DanmakuMode) -> i32 {
    match mode {
        DanmakuMode::Scroll => layout::NEXT2_TYPE_SCROLL,
        DanmakuMode::Top => layout::NEXT2_TYPE_TOP,
        DanmakuMode::Bottom => layout::NEXT2_TYPE_BOTTOM,
    }
}

fn rgba_to_argb_i32(rgba: [f32; 4]) -> i32 {
    let a = component_to_u8(rgba[3]) as u32;
    let r = component_to_u8(rgba[0]) as u32;
    let g = component_to_u8(rgba[1]) as u32;
    let b = component_to_u8(rgba[2]) as u32;
    ((a << 24) | (r << 16) | (g << 8) | b) as i32
}

fn component_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn font_size_multiplier(item_font_size: f32, base_font_size: f64) -> f64 {
    let base = if base_font_size.is_finite() && base_font_size > 0.0 {
        base_font_size
    } else {
        25.0
    };
    let item = if item_font_size.is_finite() && item_font_size > 0.0 {
        f64::from(item_font_size)
    } else {
        base
    };
    (item / base).clamp(0.25, 8.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::danmaku::DanmakuItem;

    #[test]
    fn prepares_kuroko_timeline_with_next2_tracks() {
        let mut timeline = DanmakuTimeline::default();
        timeline.extend([
            DanmakuItem {
                id: 1,
                pts: Duration::from_secs(1),
                text: "scroll".to_string(),
                mode: DanmakuMode::Scroll,
                font_size: 25.0,
                color_rgba: [1.0, 1.0, 1.0, 1.0],
            },
            DanmakuItem {
                id: 2,
                pts: Duration::from_secs(2),
                text: "top".to_string(),
                mode: DanmakuMode::Top,
                font_size: 25.0,
                color_rgba: [1.0, 0.0, 0.0, 1.0],
            },
        ]);

        let prepared = prepare_timeline_layout(
            &timeline,
            Next2TimelineConfig {
                width: 640.0,
                height: 360.0,
                scroll_duration: Duration::from_secs(6),
                ..Next2TimelineConfig::default()
            },
        )
        .unwrap();

        assert_eq!(prepared.items.len(), 2);
        assert!(prepared.track_count > 0);
        assert_eq!(prepared.items[0].type_code, layout::NEXT2_TYPE_SCROLL);
        assert_eq!(prepared.items[1].type_code, layout::NEXT2_TYPE_TOP);

        let frame = layout_prepared_frame(prepared, Duration::from_secs(2));

        assert!(!frame.items.is_empty());
        assert!(frame.items.iter().any(|item| item.text == "scroll"));
    }

    #[test]
    fn preserves_ids_and_per_item_font_size_multiplier() {
        let mut timeline = DanmakuTimeline::default();
        timeline.extend([
            DanmakuItem {
                id: 10,
                pts: Duration::from_secs(1),
                text: "base".to_string(),
                mode: DanmakuMode::Scroll,
                font_size: 25.0,
                color_rgba: [1.0, 1.0, 1.0, 1.0],
            },
            DanmakuItem {
                id: 11,
                pts: Duration::from_secs(2),
                text: "large".to_string(),
                mode: DanmakuMode::Scroll,
                font_size: 50.0,
                color_rgba: [1.0, 1.0, 1.0, 1.0],
            },
        ]);

        let prepared = prepare_timeline_layout(
            &timeline,
            Next2TimelineConfig {
                width: 640.0,
                height: 360.0,
                font_size: 25.0,
                scroll_duration: Duration::from_secs(8),
                ..Next2TimelineConfig::default()
            },
        )
        .unwrap();

        let base = prepared
            .items
            .iter()
            .find(|item| item.id == 10)
            .expect("base item is preserved");
        let large = prepared
            .items
            .iter()
            .find(|item| item.id == 11)
            .expect("large item is preserved");

        assert_eq!(base.font_size_multiplier, 1.0);
        assert_eq!(large.font_size_multiplier, 2.0);
        assert!(large.width > base.width);
        assert!(large.height > base.height);
    }
}
