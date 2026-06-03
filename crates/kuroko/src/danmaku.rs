use std::time::Duration;

use thiserror::Error;

use crate::text::TextShaper;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DanmakuError {
    #[error("invalid danmaku field: {0}")]
    InvalidField(String),
    #[error("missing danmaku text")]
    MissingText,
}

pub type Result<T> = std::result::Result<T, DanmakuError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DanmakuMode {
    Scroll,
    Top,
    Bottom,
}

impl DanmakuMode {
    fn from_bilibili_mode(value: u32) -> Self {
        match value {
            5 => Self::Top,
            4 => Self::Bottom,
            _ => Self::Scroll,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DanmakuItem {
    pub id: u64,
    pub pts: Duration,
    pub text: String,
    pub mode: DanmakuMode,
    pub font_size: f32,
    pub color_rgba: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct DanmakuLayoutBox {
    pub item_id: u64,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
    pub color_rgba: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DanmakuLayoutConfig {
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub duration: Duration,
    pub lane_gap: f32,
}

impl Default for DanmakuLayoutConfig {
    fn default() -> Self {
        Self {
            viewport_width: 1920.0,
            viewport_height: 1080.0,
            duration: Duration::from_secs(8),
            lane_gap: 2.0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DanmakuTimeline {
    items: Vec<DanmakuItem>,
}

impl DanmakuTimeline {
    pub fn push(&mut self, item: DanmakuItem) {
        self.items.push(item);
        self.items.sort_by_key(|item| item.pts);
    }

    pub fn extend<I>(&mut self, items: I)
    where
        I: IntoIterator<Item = DanmakuItem>,
    {
        self.items.extend(items);
        self.items.sort_by_key(|item| item.pts);
    }

    pub fn items(&self) -> &[DanmakuItem] {
        &self.items
    }

    pub fn active_items(&self, position: Duration, window: Duration) -> Vec<&DanmakuItem> {
        let start = position.saturating_sub(window);
        self.items
            .iter()
            .filter(|item| item.pts >= start && item.pts <= position + window)
            .collect()
    }

    pub fn layout(
        &self,
        position: Duration,
        config: DanmakuLayoutConfig,
        shaper: &TextShaper,
    ) -> Vec<DanmakuLayoutBox> {
        let active = self.active_items(position, config.duration);
        let mut scroll_lanes: Vec<Duration> = Vec::new();
        let mut top_lanes: Vec<Duration> = Vec::new();
        let mut bottom_lanes: Vec<Duration> = Vec::new();
        let mut boxes = Vec::new();

        for item in active {
            let metrics = shaper.measure(&item.text, item.font_size);
            let elapsed = position.saturating_sub(item.pts);
            if elapsed > config.duration {
                continue;
            }

            let lane_height = metrics.height + config.lane_gap;
            let lane_count = (config.viewport_height / lane_height).floor().max(1.0) as usize;
            let lane = match item.mode {
                DanmakuMode::Scroll => {
                    choose_lane(&mut scroll_lanes, lane_count, item.pts, config.duration)
                }
                DanmakuMode::Top => {
                    choose_lane(&mut top_lanes, lane_count, item.pts, Duration::from_secs(3))
                }
                DanmakuMode::Bottom => choose_lane(
                    &mut bottom_lanes,
                    lane_count,
                    item.pts,
                    Duration::from_secs(3),
                ),
            };
            let y = match item.mode {
                DanmakuMode::Scroll | DanmakuMode::Top => lane as f32 * lane_height,
                DanmakuMode::Bottom => config.viewport_height - (lane as f32 + 1.0) * lane_height,
            };
            let x = match item.mode {
                DanmakuMode::Scroll => {
                    let t = elapsed.as_secs_f32() / config.duration.as_secs_f32().max(0.001);
                    config.viewport_width - t * (config.viewport_width + metrics.width)
                }
                DanmakuMode::Top | DanmakuMode::Bottom => {
                    (config.viewport_width - metrics.width) * 0.5
                }
            };
            boxes.push(DanmakuLayoutBox {
                item_id: item.id,
                text: item.text.clone(),
                x,
                y,
                width: metrics.width,
                height: metrics.height,
                font_size: item.font_size,
                color_rgba: item.color_rgba,
            });
        }

        boxes
    }

    pub fn to_ass_script(&self, config: DanmakuLayoutConfig, shaper: &TextShaper) -> String {
        let width = config.viewport_width.max(1.0).round() as u32;
        let height = config.viewport_height.max(1.0).round() as u32;
        let mut script = format!(
            r#"[Script Info]
ScriptType: v4.00+
ScaledBorderAndShadow: yes
PlayResX: {width}
PlayResY: {height}

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Danmaku,Arial,25,&H00FFFFFF,&H000000FF,&H80000000,&H80000000,0,0,0,0,100,100,0,0,1,1,0,7,0,0,0,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
"#
        );

        let mut scroll_lanes: Vec<Duration> = Vec::new();
        let mut top_lanes: Vec<Duration> = Vec::new();
        let mut bottom_lanes: Vec<Duration> = Vec::new();
        for item in &self.items {
            let metrics = shaper.measure(&item.text, item.font_size);
            let lane_height = metrics.height + config.lane_gap;
            let lane_count = (config.viewport_height / lane_height).floor().max(1.0) as usize;
            let (lane, hold) = match item.mode {
                DanmakuMode::Scroll => (
                    choose_lane(&mut scroll_lanes, lane_count, item.pts, config.duration),
                    config.duration,
                ),
                DanmakuMode::Top => (
                    choose_lane(&mut top_lanes, lane_count, item.pts, Duration::from_secs(3)),
                    config.duration,
                ),
                DanmakuMode::Bottom => (
                    choose_lane(
                        &mut bottom_lanes,
                        lane_count,
                        item.pts,
                        Duration::from_secs(3),
                    ),
                    config.duration,
                ),
            };
            let y = match item.mode {
                DanmakuMode::Scroll | DanmakuMode::Top => lane as f32 * lane_height,
                DanmakuMode::Bottom => config.viewport_height - (lane as f32 + 1.0) * lane_height,
            };
            let start = format_ass_timestamp(item.pts);
            let end = format_ass_timestamp(item.pts + hold);
            let color = format_ass_primary_color(item.color_rgba);
            let alpha = format_ass_alpha(item.color_rgba[3]);
            let font_size = item.font_size.max(1.0).round() as u32;
            let text = escape_ass_text(&item.text);
            let override_tags = match item.mode {
                DanmakuMode::Scroll => format!(
                    r"{{\an7\fs{font_size}\c{color}\alpha{alpha}\move({},{},{},{})}}",
                    ass_coord(config.viewport_width),
                    ass_coord(y),
                    ass_coord(-metrics.width),
                    ass_coord(y)
                ),
                DanmakuMode::Top | DanmakuMode::Bottom => {
                    let x = (config.viewport_width - metrics.width) * 0.5;
                    format!(
                        r"{{\an7\fs{font_size}\c{color}\alpha{alpha}\pos({},{})}}",
                        ass_coord(x),
                        ass_coord(y)
                    )
                }
            };
            script.push_str(&format!(
                "Dialogue: 5,{start},{end},Danmaku,,0,0,0,,{override_tags}{text}\n"
            ));
        }

        script
    }
}

pub fn parse_bilibili_xml(input: &str) -> Result<Vec<DanmakuItem>> {
    let mut items = Vec::new();
    for (index, segment) in input.split("<d ").skip(1).enumerate() {
        let Some(p_start) = segment.find("p=\"") else {
            continue;
        };
        let p_rest = &segment[p_start + 3..];
        let Some(p_end) = p_rest.find('"') else {
            continue;
        };
        let fields = p_rest[..p_end].split(',').collect::<Vec<_>>();
        let Some(text_start) = segment.find('>') else {
            continue;
        };
        let Some(text_end) = segment[text_start + 1..].find("</d>") else {
            continue;
        };
        let text = decode_xml_entities(&segment[text_start + 1..text_start + 1 + text_end]);
        if text.is_empty() {
            return Err(DanmakuError::MissingText);
        }
        items.push(DanmakuItem {
            id: fields
                .get(7)
                .and_then(|value| value.parse().ok())
                .unwrap_or(index as u64 + 1),
            pts: Duration::from_secs_f64(parse_field(fields.first(), "pts")?),
            mode: DanmakuMode::from_bilibili_mode(
                fields
                    .get(1)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1),
            ),
            font_size: fields
                .get(2)
                .and_then(|value| value.parse().ok())
                .unwrap_or(25.0),
            color_rgba: color_from_bilibili_decimal(
                fields
                    .get(3)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0xffffff),
            ),
            text,
        });
    }
    Ok(items)
}

pub fn parse_json_lines(input: &str) -> Result<Vec<DanmakuItem>> {
    let mut items = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let text = json_string_value(line, "text").ok_or(DanmakuError::MissingText)?;
        items.push(DanmakuItem {
            id: json_number_value(line, "id")
                .map(|value| value as u64)
                .unwrap_or(index as u64 + 1),
            pts: Duration::from_secs_f64(json_number_value(line, "time").unwrap_or(0.0)),
            text,
            mode: match json_string_value(line, "mode").as_deref() {
                Some("top") => DanmakuMode::Top,
                Some("bottom") => DanmakuMode::Bottom,
                _ => DanmakuMode::Scroll,
            },
            font_size: json_number_value(line, "font_size").unwrap_or(25.0) as f32,
            color_rgba: json_number_value(line, "color")
                .map(|value| color_from_bilibili_decimal(value as u32))
                .unwrap_or([1.0, 1.0, 1.0, 1.0]),
        });
    }
    Ok(items)
}

fn choose_lane(
    lanes: &mut Vec<Duration>,
    lane_count: usize,
    pts: Duration,
    hold: Duration,
) -> usize {
    if lanes.len() < lane_count {
        lanes.resize(lane_count, Duration::ZERO);
    }
    for (index, available_at) in lanes.iter_mut().enumerate() {
        if *available_at <= pts {
            *available_at = pts + hold;
            return index;
        }
    }
    let (index, available_at) = lanes
        .iter_mut()
        .enumerate()
        .min_by_key(|(_, available_at)| **available_at)
        .expect("at least one lane");
    *available_at = pts + hold;
    index
}

fn parse_field(field: Option<&&str>, name: &'static str) -> Result<f64> {
    field
        .ok_or_else(|| DanmakuError::InvalidField(name.to_string()))?
        .parse::<f64>()
        .map_err(|_| DanmakuError::InvalidField(name.to_string()))
}

fn color_from_bilibili_decimal(value: u32) -> [f32; 4] {
    let red = ((value >> 16) & 0xff) as f32 / 255.0;
    let green = ((value >> 8) & 0xff) as f32 / 255.0;
    let blue = (value & 0xff) as f32 / 255.0;
    [red, green, blue, 1.0]
}

fn format_ass_timestamp(duration: Duration) -> String {
    let centis = duration.as_millis() / 10;
    let hours = centis / 360_000;
    let minutes = (centis / 6_000) % 60;
    let seconds = (centis / 100) % 60;
    let centis = centis % 100;
    format!("{hours}:{minutes:02}:{seconds:02}.{centis:02}")
}

fn format_ass_primary_color(rgba: [f32; 4]) -> String {
    let red = color_component_to_u8(rgba[0]);
    let green = color_component_to_u8(rgba[1]);
    let blue = color_component_to_u8(rgba[2]);
    format!("&H{blue:02X}{green:02X}{red:02X}&")
}

fn format_ass_alpha(alpha: f32) -> String {
    let ass_alpha = 255u8.saturating_sub(color_component_to_u8(alpha));
    format!("&H{ass_alpha:02X}&")
}

fn color_component_to_u8(component: f32) -> u8 {
    (component.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn ass_coord(value: f32) -> i32 {
    value.round() as i32
}

fn escape_ass_text(text: &str) -> String {
    text.replace('\\', "＼")
        .replace('{', "｛")
        .replace('}', "｝")
        .replace('\r', "")
        .replace('\n', " ")
}

fn decode_xml_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn json_string_value(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = line.find(&needle)?;
    let rest = &line[start + needle.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn json_number_value(line: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{key}\"");
    let start = line.find(&needle)?;
    let rest = &line[start + needle.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_returns_items_in_window() {
        let mut timeline = DanmakuTimeline::default();
        timeline.push(DanmakuItem {
            id: 1,
            pts: Duration::from_secs(10),
            text: "hello".to_string(),
            mode: DanmakuMode::Scroll,
            font_size: 24.0,
            color_rgba: [1.0, 1.0, 1.0, 1.0],
        });
        assert_eq!(
            timeline
                .active_items(Duration::from_secs(11), Duration::from_secs(2))
                .len(),
            1
        );
        assert_eq!(
            timeline
                .active_items(Duration::from_secs(20), Duration::from_secs(2))
                .len(),
            0
        );
    }

    #[test]
    fn parses_bilibili_xml_items() {
        let xml = r#"<i><d p="1.5,1,25,16711680,0,0,0,42">hello&amp;world</d><d p="2.0,5,30,255,0,0,0,43">top</d></i>"#;
        let items = parse_bilibili_xml(xml).unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, 42);
        assert_eq!(items[0].text, "hello&world");
        assert_eq!(items[0].mode, DanmakuMode::Scroll);
        assert_eq!(items[1].mode, DanmakuMode::Top);
        assert_eq!(items[0].color_rgba, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn layout_places_items_on_lanes() {
        let mut timeline = DanmakuTimeline::default();
        timeline.extend([
            DanmakuItem {
                id: 1,
                pts: Duration::from_secs(1),
                text: "first".to_string(),
                mode: DanmakuMode::Scroll,
                font_size: 24.0,
                color_rgba: [1.0; 4],
            },
            DanmakuItem {
                id: 2,
                pts: Duration::from_secs(2),
                text: "second".to_string(),
                mode: DanmakuMode::Scroll,
                font_size: 24.0,
                color_rgba: [1.0; 4],
            },
        ]);

        let boxes = timeline.layout(
            Duration::from_secs(2),
            DanmakuLayoutConfig {
                viewport_width: 320.0,
                viewport_height: 80.0,
                duration: Duration::from_secs(8),
                lane_gap: 2.0,
            },
            &TextShaper::default(),
        );

        assert_eq!(boxes.len(), 2);
        assert_ne!(boxes[0].y, boxes[1].y);
        assert_eq!(boxes[0].text, "first");
        assert_eq!(boxes[0].font_size, 24.0);
        assert_eq!(boxes[0].color_rgba, [1.0; 4]);
    }

    #[test]
    fn ass_script_preserves_danmaku_motion_and_style() {
        let mut timeline = DanmakuTimeline::default();
        timeline.extend([
            DanmakuItem {
                id: 1,
                pts: Duration::from_millis(1500),
                text: "hello".to_string(),
                mode: DanmakuMode::Scroll,
                font_size: 24.0,
                color_rgba: [1.0, 0.0, 0.0, 0.5],
            },
            DanmakuItem {
                id: 2,
                pts: Duration::from_secs(2),
                text: "top".to_string(),
                mode: DanmakuMode::Top,
                font_size: 30.0,
                color_rgba: [0.0, 0.0, 1.0, 1.0],
            },
        ]);

        let script = timeline.to_ass_script(
            DanmakuLayoutConfig {
                viewport_width: 320.0,
                viewport_height: 180.0,
                duration: Duration::from_secs(8),
                lane_gap: 2.0,
            },
            &TextShaper::default(),
        );

        assert!(script.contains("PlayResX: 320"));
        assert!(script.contains("Dialogue: 5,0:00:01.50,0:00:09.50"));
        assert!(script.contains("\\move(320,0,"));
        assert!(script.contains("\\pos("));
        assert!(script.contains("\\fs24"));
        assert!(script.contains("\\c&H0000FF&\\alpha&H7F&"));
        assert!(script.contains("\\c&HFF0000&\\alpha&H00&"));
    }

    #[test]
    fn json_lines_parse_bilibili_decimal_color() {
        let input =
            r#"{"id":9,"time":1.25,"mode":"bottom","font_size":28,"color":65280,"text":"green"}"#;

        let items = parse_json_lines(input).unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].mode, DanmakuMode::Bottom);
        assert_eq!(items[0].font_size, 28.0);
        assert_eq!(items[0].color_rgba, [0.0, 1.0, 0.0, 1.0]);
    }
}
