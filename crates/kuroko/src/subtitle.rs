use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SubtitleError {
    #[error("invalid subtitle timestamp: {0}")]
    InvalidTimestamp(String),
    #[error("invalid subtitle cue")]
    InvalidCue,
}

pub type Result<T> = std::result::Result<T, SubtitleError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleTrackConfig {
    pub id: i64,
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleCue {
    pub start: Duration,
    pub end: Duration,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleBitmapPlane {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleFrame {
    pub pts: Duration,
    pub planes: Vec<SubtitleBitmapPlane>,
}

#[derive(Debug, Clone, Default)]
pub struct SubtitleTimeline {
    cues: Vec<SubtitleCue>,
}

impl SubtitleTimeline {
    pub fn new(cues: Vec<SubtitleCue>) -> Self {
        let mut timeline = Self { cues };
        timeline.cues.sort_by_key(|cue| cue.start);
        timeline
    }

    pub fn cues(&self) -> &[SubtitleCue] {
        &self.cues
    }

    pub fn active_cues(&self, pts: Duration) -> Vec<&SubtitleCue> {
        self.cues
            .iter()
            .filter(|cue| cue.start <= pts && pts < cue.end)
            .collect()
    }

    pub fn render_debug_frame(&self, pts: Duration, width: u32, height: u32) -> SubtitleFrame {
        let active = self.active_cues(pts);
        let mut planes = Vec::new();
        for (index, cue) in active.iter().enumerate() {
            let text_width = (cue.text.chars().count() as u32).saturating_mul(10).max(16);
            let plane_width = text_width.min(width.max(1));
            let plane_height = 28u32.min(height.max(1));
            let x = ((width.saturating_sub(plane_width)) / 2) as i32;
            let y = height
                .saturating_sub(plane_height.saturating_mul(index as u32 + 1))
                .saturating_sub(24) as i32;
            planes.push(SubtitleBitmapPlane {
                x,
                y,
                width: plane_width,
                height: plane_height,
                rgba: debug_rgba_plane(plane_width, plane_height),
            });
        }
        SubtitleFrame { pts, planes }
    }
}

pub fn parse_srt(input: &str) -> Result<SubtitleTimeline> {
    let mut cues = Vec::new();
    for block in input.replace("\r\n", "\n").split("\n\n") {
        let lines = block
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        if lines.is_empty() {
            continue;
        }
        let time_line_index = usize::from(!lines[0].contains("-->"));
        let Some(time_line) = lines.get(time_line_index) else {
            continue;
        };
        if !time_line.contains("-->") {
            continue;
        }
        let text = lines[time_line_index + 1..].join("\n");
        cues.push(parse_timed_text_cue(time_line, &text)?);
    }
    Ok(SubtitleTimeline::new(cues))
}

pub fn parse_webvtt(input: &str) -> Result<SubtitleTimeline> {
    let normalized = input.replace("\r\n", "\n");
    let without_header = normalized.strip_prefix("WEBVTT").unwrap_or(&normalized);
    parse_srt(without_header)
}

pub fn parse_ass_events(input: &str) -> Result<SubtitleTimeline> {
    let mut in_events = false;
    let mut format_fields = Vec::new();
    let mut cues = Vec::new();

    for line in input.lines() {
        let line = line.trim();
        if line.eq_ignore_ascii_case("[events]") {
            in_events = true;
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(format) = line.strip_prefix("Format:") {
            format_fields = format
                .split(',')
                .map(|field| field.trim().to_ascii_lowercase())
                .collect();
            continue;
        }
        if let Some(dialogue) = line.strip_prefix("Dialogue:") {
            let field_count = format_fields.len().max(10);
            let values = dialogue
                .splitn(field_count, ',')
                .map(str::trim)
                .collect::<Vec<_>>();
            let index = |name: &str, default: usize| {
                format_fields
                    .iter()
                    .position(|field| field == name)
                    .unwrap_or(default)
            };
            let start = values
                .get(index("start", 1))
                .ok_or(SubtitleError::InvalidCue)
                .and_then(|value| parse_timestamp(value))?;
            let end = values
                .get(index("end", 2))
                .ok_or(SubtitleError::InvalidCue)
                .and_then(|value| parse_timestamp(value))?;
            let text = values
                .get(index("text", 9))
                .map(|value| clean_ass_text(value))
                .unwrap_or_default();
            cues.push(SubtitleCue { start, end, text });
        }
    }

    Ok(SubtitleTimeline::new(cues))
}

fn parse_timed_text_cue(time_line: &str, text: &str) -> Result<SubtitleCue> {
    let (start, end) = time_line
        .split_once("-->")
        .ok_or(SubtitleError::InvalidCue)?;
    let start = parse_timestamp(start.trim())?;
    let end_part = end.split_whitespace().next().unwrap_or(end).trim();
    let end = parse_timestamp(end_part)?;
    Ok(SubtitleCue {
        start,
        end,
        text: text.trim().to_string(),
    })
}

fn parse_timestamp(value: &str) -> Result<Duration> {
    let value = value.trim().replace(',', ".");
    let parts = value.split(':').collect::<Vec<_>>();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0u64, parse_int(minutes)?, parse_seconds(seconds)?),
        [hours, minutes, seconds] => (
            parse_int(hours)?,
            parse_int(minutes)?,
            parse_seconds(seconds)?,
        ),
        _ => return Err(SubtitleError::InvalidTimestamp(value)),
    };
    Ok(Duration::from_secs(hours * 3600 + minutes * 60) + seconds)
}

fn parse_int(value: &str) -> Result<u64> {
    value
        .parse::<u64>()
        .map_err(|_| SubtitleError::InvalidTimestamp(value.to_string()))
}

fn parse_seconds(value: &str) -> Result<Duration> {
    let seconds = value
        .parse::<f64>()
        .map_err(|_| SubtitleError::InvalidTimestamp(value.to_string()))?;
    Ok(Duration::from_secs_f64(seconds))
}

fn clean_ass_text(value: &str) -> String {
    let mut output = String::new();
    let mut in_override = false;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => in_override = true,
            '}' => in_override = false,
            _ if in_override => {}
            '\\' => match chars.peek().copied() {
                Some('N') | Some('n') => {
                    let _ = chars.next();
                    output.push('\n');
                }
                _ => output.push(ch),
            },
            _ => output.push(ch),
        }
    }
    output.trim().to_string()
}

fn debug_rgba_plane(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[255, 255, 255, 220]);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_srt_and_finds_active_cue() {
        let srt =
            "1\n00:00:01,000 --> 00:00:03,500\nHello\n\n2\n00:00:04,000 --> 00:00:05,000\nWorld\n";
        let timeline = parse_srt(srt).unwrap();

        assert_eq!(timeline.cues().len(), 2);
        assert_eq!(
            timeline.active_cues(Duration::from_millis(1500))[0].text,
            "Hello"
        );
        assert!(timeline.active_cues(Duration::from_millis(3500)).is_empty());
    }

    #[test]
    fn parses_ass_dialogue() {
        let ass = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\i1}Hi\\Nthere";
        let timeline = parse_ass_events(ass).unwrap();

        assert_eq!(timeline.cues().len(), 1);
        assert_eq!(timeline.cues()[0].text, "Hi\nthere");
    }

    #[test]
    fn debug_frame_produces_rgba_planes() {
        let timeline = SubtitleTimeline::new(vec![SubtitleCue {
            start: Duration::from_secs(1),
            end: Duration::from_secs(3),
            text: "Hello".to_string(),
        }]);

        let frame = timeline.render_debug_frame(Duration::from_secs(2), 640, 360);

        assert_eq!(frame.planes.len(), 1);
        assert_eq!(
            frame.planes[0].rgba.len(),
            frame.planes[0].width as usize * frame.planes[0].height as usize * 4
        );
    }
}
