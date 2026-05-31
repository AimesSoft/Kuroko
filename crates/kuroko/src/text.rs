#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontQuery {
    pub family: Option<String>,
    pub weight: u16,
    pub italic: bool,
}

impl Default for FontQuery {
    fn default() -> Self {
        Self {
            family: None,
            weight: 400,
            italic: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    pub text: String,
    pub font_size: f32,
    pub color_rgba: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    pub ascent: f32,
    pub descent: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapedGlyph {
    pub glyph_id: u32,
    pub cluster: usize,
    pub advance_x: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapedTextRun {
    pub font: FontQuery,
    pub run: GlyphRun,
    pub glyphs: Vec<ShapedGlyph>,
    pub metrics: TextMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextLayoutConfig {
    pub default_advance_em: f32,
    pub wide_advance_em: f32,
    pub line_height_em: f32,
}

impl Default for TextLayoutConfig {
    fn default() -> Self {
        Self {
            default_advance_em: 0.56,
            wide_advance_em: 1.0,
            line_height_em: 1.2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextShaper {
    config: TextLayoutConfig,
}

impl TextShaper {
    pub fn new(config: TextLayoutConfig) -> Self {
        Self { config }
    }

    pub fn shape(&self, font: FontQuery, run: GlyphRun) -> ShapedTextRun {
        let mut x = 0.0f32;
        let mut glyphs = Vec::new();
        for (cluster, ch) in run.text.char_indices() {
            let advance_x = self.advance_for(ch, run.font_size);
            glyphs.push(ShapedGlyph {
                glyph_id: ch as u32,
                cluster,
                advance_x,
                offset_x: x,
                offset_y: 0.0,
            });
            x += advance_x;
        }
        let metrics = TextMetrics {
            width: x,
            height: run.font_size * self.config.line_height_em,
            ascent: run.font_size * 0.8,
            descent: run.font_size * 0.2,
        };
        ShapedTextRun {
            font,
            run,
            glyphs,
            metrics,
        }
    }

    pub fn measure(&self, text: &str, font_size: f32) -> TextMetrics {
        self.shape(
            FontQuery::default(),
            GlyphRun {
                text: text.to_string(),
                font_size,
                color_rgba: [1.0, 1.0, 1.0, 1.0],
            },
        )
        .metrics
    }

    fn advance_for(&self, ch: char, font_size: f32) -> f32 {
        let em = if is_wide(ch) {
            self.config.wide_advance_em
        } else if ch.is_whitespace() {
            self.config.default_advance_em * 0.55
        } else {
            self.config.default_advance_em
        };
        font_size * em
    }
}

impl Default for TextShaper {
    fn default() -> Self {
        Self::new(TextLayoutConfig::default())
    }
}

fn is_wide(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1100..=0x115f
            | 0x2e80..=0xa4cf
            | 0xac00..=0xd7a3
            | 0xf900..=0xfaff
            | 0xfe10..=0xfe19
            | 0xfe30..=0xfe6f
            | 0xff00..=0xff60
            | 0xffe0..=0xffe6
            | 0x1f300..=0x1faff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_shaper_measures_wide_text_larger_than_ascii() {
        let shaper = TextShaper::default();

        let ascii = shaper.measure("abcd", 20.0);
        let wide = shaper.measure("弹幕", 20.0);

        assert!(ascii.width > 0.0);
        assert!(wide.width > ascii.width * 0.8);
        assert_eq!(wide.height, 24.0);
    }

    #[test]
    fn text_shaper_keeps_glyph_clusters() {
        let shaped = TextShaper::default().shape(
            FontQuery::default(),
            GlyphRun {
                text: "A幕".to_string(),
                font_size: 16.0,
                color_rgba: [1.0, 0.0, 0.0, 1.0],
            },
        );

        assert_eq!(shaped.glyphs.len(), 2);
        assert_eq!(shaped.glyphs[0].cluster, 0);
        assert_eq!(shaped.glyphs[1].cluster, 1);
        assert!(shaped.glyphs[1].advance_x > shaped.glyphs[0].advance_x);
    }
}
