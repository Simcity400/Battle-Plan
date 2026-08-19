
use crate::font::{strip_color_codes, BitmapFont};
use std::fmt;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineTextBorder {
    Authored { x: i32, y: i32 },
    ExplicitlyAbsent,
}

impl EngineTextBorder {
    fn components(self) -> Result<(i32, i32), EngineTextLayoutError> {
        match self {
            Self::Authored { x, y } if x < 0 || y < 0 => {
                Err(EngineTextLayoutError::NegativeAuthoredBorder { x, y })
            }
            Self::Authored { x, y } => Ok((x, y)),
            Self::ExplicitlyAbsent => Ok((0, 0)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineTextLayoutError {
    NonPositiveLineHeight {
        line_height: i32,
    },
    NegativeAuthoredBorder {
        x: i32,
        y: i32,
    },
    InvalidWrapWidth {
        max_width: i32,
        border_x: i32,
    },
    MissingGlyph {
        ch: char,
        byte_index: usize,
    },
    InlineIconRequiresResolvedMetrics {
        byte_index: usize,
    },
    InlineIconMetricsMismatch {
        ordinal: usize,
        token: String,
    },
}

impl fmt::Display for EngineTextLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonPositiveLineHeight { line_height } => {
                write!(
                    f,
                    "bitmap font declares non-positive lineHeight {line_height}"
                )
            }
            Self::NegativeAuthoredBorder { x, y } => {
                write!(f, "authored text borderSize is negative: ({x}, {y})")
            }
            Self::InvalidWrapWidth {
                max_width,
                border_x,
            } => write!(
                f,
                "text wrap width max_width - 2*border_x is non-positive or outside i32: \
                 max_width={max_width}, border_x={border_x}"
            ),
            Self::MissingGlyph { ch, byte_index } => write!(
                f,
                "bitmap font has no glyph for U+{:04X} at stripped byte {byte_index}",
                *ch as u32
            ),
            Self::InlineIconRequiresResolvedMetrics { byte_index } => write!(
                f,
                "inline icon at stripped byte {byte_index} requires admitted sprite advance, \
                 height, baseline and frame"
            ),
            Self::InlineIconMetricsMismatch { ordinal, token } => write!(
                f,
                "inline icon #{ordinal} `{token}` does not match one complete admitted \
                 sprite-metrics record"
            ),
        }
    }
}

impl std::error::Error for EngineTextLayoutError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineTextGlyph {
    pub ch: char,
    pub visible_char_index: usize,
    pub byte_range: Range<usize>,
    pub line_index: usize,
    pub pen_x: i32,
    pub pen_y: i32,
    pub baseline_y: i32,
    pub draw_x: i32,
    pub draw_y: i32,
    pub width: usize,
    pub height: usize,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    pub kerning: i32,
    pub page: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineTextLine {
    pub byte_range: Range<usize>,
    pub glyph_range: Range<usize>,
    pub inline_icon_range: Range<usize>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineTextLayout {
    pub stripped_text: String,
    pub wrap_width: i32,
    pub content_width: i32,
    pub content_height: i32,
    pub total_width: i32,
    pub total_height: i32,
    pub lines: Vec<EngineTextLine>,
    pub glyphs: Vec<EngineTextGlyph>,
    pub inline_icons: Vec<EngineTextInlineIcon>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedInlineIconMetrics {
    pub token: String,
    pub sprite: String,
    pub frame: u32,
    pub advance: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineTextInlineIcon {
    pub token: String,
    pub sprite: String,
    pub frame: u32,
    pub byte_range: Range<usize>,
    pub line_index: usize,
    pub x: i32,
    pub line_y: i32,
    pub engine_center_offset_y: i32,
    pub advance: i32,
    pub height: i32,
}

pub fn engine_inline_icon_frame_index(authored_frame: Option<u32>) -> Option<u32> {
    match authored_frame {
        None => Some(0),
        Some(frame) => frame.checked_sub(1),
    }
}

pub fn layout_engine_text(
    font: &BitmapFont,
    text: &str,
    max_width: i32,
    border: EngineTextBorder,
) -> Result<EngineTextLayout, EngineTextLayoutError> {
    layout_engine_text_with_icons(font, text, max_width, border, &[])
}

pub fn layout_engine_text_with_icons(
    font: &BitmapFont,
    text: &str,
    max_width: i32,
    border: EngineTextBorder,
    resolved_icons: &[ResolvedInlineIconMetrics],
) -> Result<EngineTextLayout, EngineTextLayoutError> {
    if font.line_height <= 0 {
        return Err(EngineTextLayoutError::NonPositiveLineHeight {
            line_height: font.line_height,
        });
    }
    let (border_x, border_y) = border.components()?;
    let wrap_width_i64 = i64::from(max_width) - i64::from(border_x) * 2;
    let wrap_width = i32::try_from(wrap_width_i64)
        .ok()
        .filter(|width| *width > 0)
        .ok_or(EngineTextLayoutError::InvalidWrapWidth {
            max_width,
            border_x,
        })?;

    let stripped_text = strip_color_codes(text);
    let mut glyphs = Vec::<EngineTextGlyph>::new();
    let mut inline_icons = Vec::<EngineTextInlineIcon>::new();
    let mut lines = Vec::<EngineTextLine>::new();
    let mut line_start_byte = 0usize;
    let mut line_start_glyph = 0usize;
    let mut line_start_icon = 0usize;
    let mut line_index = 0usize;
    let mut line_y = border_y;
    let mut line_width = 0i32;
    let mut word_width = 0i32;
    let mut committed_line_height = font.line_height;
    let mut word_height = font.line_height;
    let mut trailing_word_carry = 0i32;
    let mut word_start_byte = 0usize;
    let mut word_start_glyph = 0usize;
    let mut word_start_icon = 0usize;
    let mut previous_glyph_id = None::<u32>;
    let mut color_char_index = 0usize;
    let mut characters = stripped_text.char_indices().peekable();

    while let Some((byte_index, ch)) = characters.next() {
        let next_byte = byte_index.saturating_add(ch.len_utf8());
        if ch == '\n' || ch == '\r' {
            color_char_index = color_char_index.saturating_add(1);
            let line_height = committed_line_height.max(word_height);
            finish_line(
                &mut lines,
                line_start_byte..byte_index,
                line_start_glyph..glyphs.len(),
                line_start_icon..inline_icons.len(),
                border_x,
                line_y,
                line_width,
                line_height,
            );
            line_index = line_index.saturating_add(1);
            line_y = line_y.saturating_add(line_height);
            line_start_byte = next_byte;
            line_start_glyph = glyphs.len();
            line_start_icon = inline_icons.len();
            line_width = 0;
            word_width = 0;
            committed_line_height = font.line_height;
            word_height = font.line_height;
            trailing_word_carry = 0;
            word_start_byte = next_byte;
            word_start_glyph = glyphs.len();
            word_start_icon = inline_icons.len();
            previous_glyph_id = None;
            continue;
        }
        if ch == '\u{00a3}' {
            let token_end = inline_icon_token_end(&stripped_text, next_byte);
            while characters
                .peek()
                .is_some_and(|(next_index, _)| *next_index < token_end)
            {
                characters.next();
            }
            let token = stripped_text[next_byte..token_end].to_string();
            let ordinal = inline_icons.len();
            let metrics = resolved_icons
                .get(ordinal)
                .ok_or(EngineTextLayoutError::InlineIconRequiresResolvedMetrics { byte_index })?;
            let declared_frame_matches = token
                .rsplit_once('|')
                .map(|(_, digits)| {
                    !digits.is_empty()
                        && digits
                            .parse::<u32>()
                            .ok()
                            .and_then(|frame| engine_inline_icon_frame_index(Some(frame)))
                            == Some(metrics.frame)
                })
                .unwrap_or(metrics.frame == 0);
            if token.is_empty()
                || metrics.token != token
                || !declared_frame_matches
                || metrics.sprite.trim().is_empty()
                || metrics.advance <= 0
                || metrics.height <= 0
            {
                return Err(EngineTextLayoutError::InlineIconMetricsMismatch { ordinal, token });
            }
            wrap_before_item(
                &mut lines,
                &mut glyphs,
                &mut inline_icons,
                &mut line_start_byte,
                &mut line_start_glyph,
                &mut line_start_icon,
                &mut line_index,
                &mut line_y,
                &mut line_width,
                &mut word_width,
                &mut committed_line_height,
                &mut word_height,
                &mut trailing_word_carry,
                &mut word_start_byte,
                &mut word_start_glyph,
                &mut word_start_icon,
                byte_index,
                metrics.advance,
                wrap_width,
                border_x,
                font.line_height,
                font.base,
            );
            inline_icons.push(EngineTextInlineIcon {
                token,
                sprite: metrics.sprite.clone(),
                frame: metrics.frame,
                byte_range: byte_index..token_end,
                line_index,
                x: border_x.saturating_add(line_width),
                line_y,
                engine_center_offset_y: 0,
                advance: metrics.advance,
                height: metrics.height,
            });
            line_width = line_width.saturating_add(metrics.advance);
            word_width = word_width.saturating_add(metrics.advance);
            word_height = word_height.max(metrics.height);
            continue;
        }

        let id = ch as u32;
        let font_char = font
            .chars
            .get(&id)
            .ok_or(EngineTextLayoutError::MissingGlyph { ch, byte_index })?;
        let kerning = previous_glyph_id
            .and_then(|previous| font.kernings.get(&(previous, id)).copied())
            .unwrap_or(0);
        let advance = kerning.saturating_add(font_char.xadvance);
        let visible_char_index = color_char_index;
        color_char_index = color_char_index.saturating_add(1);

        let may_wrap_here = font_char.height != 0 || word_width == 0;
        if may_wrap_here && line_width.saturating_add(advance) >= wrap_width {
            wrap_before_item(
                &mut lines,
                &mut glyphs,
                &mut inline_icons,
                &mut line_start_byte,
                &mut line_start_glyph,
                &mut line_start_icon,
                &mut line_index,
                &mut line_y,
                &mut line_width,
                &mut word_width,
                &mut committed_line_height,
                &mut word_height,
                &mut trailing_word_carry,
                &mut word_start_byte,
                &mut word_start_glyph,
                &mut word_start_icon,
                byte_index,
                advance,
                wrap_width,
                border_x,
                font.line_height,
                font.base,
            );
        }

        let pen_x = border_x.saturating_add(line_width).saturating_add(kerning);
        glyphs.push(EngineTextGlyph {
            ch,
            visible_char_index,
            byte_range: byte_index..next_byte,
            line_index,
            pen_x,
            pen_y: line_y,
            baseline_y: line_y.saturating_add(font.base),
            draw_x: pen_x.saturating_add(font_char.xoffset),
            draw_y: line_y.saturating_add(font_char.yoffset),
            width: font_char.width,
            height: font_char.height,
            xoffset: font_char.xoffset,
            yoffset: font_char.yoffset,
            xadvance: font_char.xadvance,
            kerning,
            page: font_char.page,
        });
        line_width = line_width.saturating_add(advance);
        word_width = word_width.saturating_add(advance);
        previous_glyph_id = Some(id);

        if font_char.height == 0 {
            committed_line_height = committed_line_height.max(word_height);
            word_height = font.line_height;
            word_width = 0;
            trailing_word_carry = advance;
            word_start_byte = next_byte;
            word_start_glyph = glyphs.len();
            word_start_icon = inline_icons.len();
        }
    }

    if inline_icons.len() != resolved_icons.len() {
        let ordinal = inline_icons.len();
        let token = resolved_icons
            .get(ordinal)
            .map(|metrics| metrics.token.clone())
            .unwrap_or_default();
        return Err(EngineTextLayoutError::InlineIconMetricsMismatch { ordinal, token });
    }
    let final_line_height = committed_line_height.max(word_height);
    finish_line(
        &mut lines,
        line_start_byte..stripped_text.len(),
        line_start_glyph..glyphs.len(),
        line_start_icon..inline_icons.len(),
        border_x,
        line_y,
        line_width,
        final_line_height,
    );
    for icon in &mut inline_icons {
        let line_height = lines
            .get(icon.line_index)
            .map(|line| line.height)
            .unwrap_or(font.line_height);
        icon.engine_center_offset_y = icon.height / 2 - line_height / 2;
    }

    let content_width = lines
        .iter()
        .map(|line| line.width)
        .max()
        .unwrap_or(0)
        .max(0);
    let content_height = lines
        .iter()
        .fold(0i32, |height, line| height.saturating_add(line.height));
    let total_width = content_width.saturating_add(border_x.saturating_mul(2));
    let total_height = content_height.saturating_add(border_y.saturating_mul(2));
    Ok(EngineTextLayout {
        stripped_text,
        wrap_width,
        content_width,
        content_height,
        total_width,
        total_height,
        lines,
        glyphs,
        inline_icons,
    })
}

#[allow(clippy::too_many_arguments)]
fn wrap_before_item(
    lines: &mut Vec<EngineTextLine>,
    glyphs: &mut [EngineTextGlyph],
    inline_icons: &mut [EngineTextInlineIcon],
    line_start_byte: &mut usize,
    line_start_glyph: &mut usize,
    line_start_icon: &mut usize,
    line_index: &mut usize,
    line_y: &mut i32,
    line_width: &mut i32,
    word_width: &mut i32,
    committed_line_height: &mut i32,
    word_height: &mut i32,
    trailing_word_carry: &mut i32,
    word_start_byte: &mut usize,
    word_start_glyph: &mut usize,
    word_start_icon: &mut usize,
    item_byte_index: usize,
    item_advance: i32,
    wrap_width: i32,
    border_x: i32,
    default_line_height: i32,
    base: i32,
) {
    if (*line_width).saturating_add(item_advance) < wrap_width {
        return;
    }
    if *line_width == *word_width {
        let height = (*committed_line_height).max(*word_height);
        finish_line(
            lines,
            *line_start_byte..item_byte_index,
            *line_start_glyph..glyphs.len(),
            *line_start_icon..inline_icons.len(),
            border_x,
            *line_y,
            *line_width,
            height,
        );
        *line_index = (*line_index).saturating_add(1);
        *line_y = (*line_y).saturating_add(height);
        *line_start_byte = item_byte_index;
        *line_start_glyph = glyphs.len();
        *line_start_icon = inline_icons.len();
        *line_width = 0;
        *word_width = 0;
        *committed_line_height = default_line_height;
        *word_height = default_line_height;
        *trailing_word_carry = 0;
        *word_start_byte = item_byte_index;
        *word_start_glyph = glyphs.len();
        *word_start_icon = inline_icons.len();
    } else {
        let recorded_width = (*line_width)
            .saturating_sub(*word_width)
            .saturating_sub(*trailing_word_carry);
        finish_line(
            lines,
            *line_start_byte..*word_start_byte,
            *line_start_glyph..*word_start_glyph,
            *line_start_icon..*word_start_icon,
            border_x,
            *line_y,
            recorded_width,
            *committed_line_height,
        );
        *line_index = (*line_index).saturating_add(1);
        *line_y = (*line_y).saturating_add(*committed_line_height);
        let horizontal_shift = (*word_width).saturating_sub(*line_width);
        shift_word_to_line(
            &mut glyphs[*word_start_glyph..],
            &mut inline_icons[*word_start_icon..],
            *line_index,
            horizontal_shift,
            *line_y,
            base,
        );
        *line_start_byte = *word_start_byte;
        *line_start_glyph = *word_start_glyph;
        *line_start_icon = *word_start_icon;
        *line_width = *word_width;
        *committed_line_height = *word_height;
        *trailing_word_carry = 0;
    }
}

fn finish_line(
    lines: &mut Vec<EngineTextLine>,
    byte_range: Range<usize>,
    glyph_range: Range<usize>,
    inline_icon_range: Range<usize>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    lines.push(EngineTextLine {
        byte_range,
        glyph_range,
        inline_icon_range,
        x,
        y,
        width: width.max(0),
        height,
    });
}

fn shift_word_to_line(
    glyphs: &mut [EngineTextGlyph],
    inline_icons: &mut [EngineTextInlineIcon],
    line_index: usize,
    horizontal_shift: i32,
    line_y: i32,
    base: i32,
) {
    for glyph in glyphs {
        glyph.line_index = line_index;
        glyph.pen_x = glyph.pen_x.saturating_add(horizontal_shift);
        glyph.pen_y = line_y;
        glyph.baseline_y = line_y.saturating_add(base);
        glyph.draw_x = glyph.pen_x.saturating_add(glyph.xoffset);
        glyph.draw_y = line_y.saturating_add(glyph.yoffset);
    }
    for icon in inline_icons {
        icon.line_index = line_index;
        icon.x = icon.x.saturating_add(horizontal_shift);
        icon.line_y = line_y;
    }
}

fn inline_icon_token_end(text: &str, token_start: usize) -> usize {
    let mut end = token_start;
    for (offset, ch) in text[token_start..].char_indices() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            end = token_start
                .saturating_add(offset)
                .saturating_add(ch.len_utf8());
        } else {
            break;
        }
    }
    if text.as_bytes().get(end) == Some(&b'|') {
        let mut frame_end = end.saturating_add(1);
        for (offset, ch) in text[frame_end..].char_indices() {
            if ch.is_ascii_digit() {
                frame_end = frame_end.max(
                    end.saturating_add(1)
                        .saturating_add(offset)
                        .saturating_add(1),
                );
            } else {
                break;
            }
        }
        end = frame_end;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{BitmapFont, FontChar};
    use std::collections::BTreeMap;

    fn font() -> BitmapFont {
        let mut chars = BTreeMap::new();
        for ch in ['A', 'B', '_', ' ', '\t'] {
            let zero_extent = matches!(ch, '_' | ' ' | '\t');
            chars.insert(
                ch as u32,
                FontChar {
                    id: ch as u32,
                    x: 0,
                    y: 0,
                    width: if zero_extent { 0 } else { 5 },
                    height: if zero_extent { 0 } else { 8 },
                    xoffset: 1,
                    yoffset: 2,
                    xadvance: if zero_extent { 3 } else { 6 },
                    page: 0,
                    chnl: 15,
                },
            );
        }
        BitmapFont {
            name: "engine-layout-test".into(),
            source_file: "synthetic.fnt".into(),
            info_size: 9,
            info_padding: [0; 4],
            info_spacing: [0; 2],
            outline: 0,
            line_height: 9,
            base: 7,
            scale_w: 1,
            scale_h: 1,
            glyph_channel: None,
            pages: Vec::new(),
            chars,
            kernings: BTreeMap::from([(('A' as u32, 'B' as u32), -2)]),
            color_table: None,
            authored_base_color_rgba8: None,
            authored_base_border_color_rgba8: None,
        }
    }

    fn line_texts(layout: &EngineTextLayout) -> Vec<&str> {
        layout
            .lines
            .iter()
            .map(|line| &layout.stripped_text[line.byte_range.clone()])
            .collect()
    }

    #[test]
    fn a_non_positive_declared_line_height_refuses() {
        for line_height in [0, -1, i32::MIN] {
            let mut broken = font();
            broken.line_height = line_height;
            let error = layout_engine_text(
                &broken,
                "AA",
                100,
                EngineTextBorder::Authored { x: 2, y: 1 },
            )
            .expect_err("a font declaring no line advance must fail closed");
            assert_eq!(
                error,
                EngineTextLayoutError::NonPositiveLineHeight { line_height },
            );
            let with_absent_border =
                layout_engine_text(&broken, "AA", 100, EngineTextBorder::ExplicitlyAbsent)
                    .expect_err("still refuses without an authored border");
            assert_eq!(
                with_absent_border,
                EngineTextLayoutError::NonPositiveLineHeight { line_height },
            );
            assert!(error
                .to_string()
                .contains(&format!("non-positive lineHeight {line_height}")));
        }
        assert!(layout_engine_text(
            &font(),
            "AA",
            100,
            EngineTextBorder::Authored { x: 2, y: 1 }
        )
        .is_ok());
    }

    #[test]
    fn wraps_the_whole_word_at_a_zero_extent_glyph() {
        let layout = layout_engine_text(
            &font(),
            "AA BB",
            22,
            EngineTextBorder::Authored { x: 2, y: 1 },
        )
        .expect("synthetic font covers the text");
        assert_eq!(layout.wrap_width, 18);
        assert_eq!(line_texts(&layout), vec!["AA ", "BB"]);
        assert_eq!(layout.lines[0].width, 12);
        assert_eq!(layout.glyphs[3].pen_x, 2);
    }

    #[test]
    fn repeated_and_edge_whitespace_survives_byte_for_byte() {
        let layout = layout_engine_text(&font(), "  AA  ", 100, EngineTextBorder::ExplicitlyAbsent)
            .expect("spaces are mapped");
        assert_eq!(layout.stripped_text, "  AA  ");
        assert_eq!(line_texts(&layout), vec!["  AA  "]);
        assert_eq!(
            layout
                .glyphs
                .iter()
                .map(|glyph| glyph.ch)
                .collect::<String>(),
            "  AA  "
        );
    }

    #[test]
    fn zero_extent_non_whitespace_is_the_break_candidate() {
        let layout = layout_engine_text(&font(), "AA_BB", 18, EngineTextBorder::ExplicitlyAbsent)
            .expect("underscore is mapped");
        assert_eq!(line_texts(&layout), vec!["AA_", "BB"]);
        assert_eq!(
            layout
                .glyphs
                .iter()
                .map(|glyph| glyph.ch)
                .collect::<String>(),
            "AA_BB"
        );
    }

    #[test]
    fn crlf_is_two_breaks_and_trailing_break_retains_the_empty_line() {
        let layout =
            layout_engine_text(&font(), "A\r\nB\r", 100, EngineTextBorder::ExplicitlyAbsent)
                .expect("A and B are mapped");
        assert_eq!(line_texts(&layout), vec!["A", "", "B", ""]);
        let pens = layout
            .glyphs
            .iter()
            .map(|glyph| (glyph.ch, glyph.pen_y))
            .collect::<Vec<_>>();
        assert_eq!(pens, vec![('A', 0), ('B', font().line_height * 2)]);
    }

    #[test]
    fn kerning_comes_from_the_previous_glyph_pair_before_xadvance() {
        let layout = layout_engine_text(&font(), "AB", 100, EngineTextBorder::ExplicitlyAbsent)
            .expect("A and B are mapped");
        assert_eq!(layout.glyphs[1].kerning, -2);
        assert_eq!(layout.glyphs[1].pen_x, 4);
        assert_eq!(layout.lines[0].width, 10);
    }

    #[test]
    fn section_code_mid_word_neither_moves_the_break_nor_desynchronises_colour_indices() {
        let marked = "AA\u{00a7}RBB AA";
        let plain = "AABB AA";
        let marked_layout =
            layout_engine_text(&font(), marked, 26, EngineTextBorder::ExplicitlyAbsent)
                .expect("visible glyphs are mapped");
        let plain_layout =
            layout_engine_text(&font(), plain, 26, EngineTextBorder::ExplicitlyAbsent)
                .expect("visible glyphs are mapped");
        assert_eq!(line_texts(&marked_layout), line_texts(&plain_layout));
        assert_eq!(line_texts(&marked_layout), vec!["AABB ", "AA"]);
        let colors = font().colorize(marked, [1, 2, 3, 4]).colors;
        assert_eq!(colors.len(), marked_layout.glyphs.len());
        assert_eq!(
            marked_layout
                .glyphs
                .iter()
                .map(|glyph| glyph.visible_char_index)
                .collect::<Vec<_>>(),
            (0..colors.len()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn missing_controls_are_typed_refusals_not_silent_skips() {
        let error = layout_engine_text(
            &font(),
            "A\u{0001}B",
            100,
            EngineTextBorder::ExplicitlyAbsent,
        )
        .expect_err("unmapped control must refuse");
        assert_eq!(
            error,
            EngineTextLayoutError::MissingGlyph {
                ch: '\u{0001}',
                byte_index: 1
            }
        );
    }

    #[test]
    fn resolved_inline_icon_supplies_advance_decoded_frame_and_tall_line_box() {
        let metrics = [ResolvedInlineIconMetrics {
            token: "tank|2".into(),
            sprite: "GFX_text_tank".into(),
            frame: 1,
            advance: 10,
            height: 15,
        }];
        let layout = layout_engine_text_with_icons(
            &font(),
            "A\u{00a3}tank|2B\rA",
            100,
            EngineTextBorder::ExplicitlyAbsent,
            &metrics,
        )
        .expect("all icon metrics are admitted");
        assert_eq!(line_texts(&layout), vec!["A\u{00a3}tank|2B", "A"]);
        assert_eq!(layout.lines[0].height, 15);
        assert_eq!(layout.lines[1].y, 15);
        assert_eq!(layout.content_height, 24);
        assert_eq!(
            layout.inline_icons,
            vec![EngineTextInlineIcon {
                token: "tank|2".into(),
                sprite: "GFX_text_tank".into(),
                frame: 1,
                byte_range: 1..9,
                line_index: 0,
                x: 6,
                line_y: 0,
                engine_center_offset_y: 0,
                advance: 10,
                height: 15,
            }]
        );
        assert_eq!(
            layout.glyphs[1].pen_x, 14,
            "the special-element branch preserves the previous glyph, so A-B kerning \
             remains in force across the inline icon"
        );
        assert_eq!(layout.glyphs[1].kerning, -2);
    }

    #[test]
    fn inline_icon_tokens_do_not_shift_following_glyph_colour_indices() {
        let metrics = [ResolvedInlineIconMetrics {
            token: "tank|2".into(),
            sprite: "GFX_text_tank".into(),
            frame: 1,
            advance: 10,
            height: 15,
        }];
        let marked = "\u{00a7}RA\u{00a3}tank|2\u{00a7}GB";
        let layout = layout_engine_text_with_icons(
            &font(),
            marked,
            100,
            EngineTextBorder::ExplicitlyAbsent,
            &metrics,
        )
        .expect("glyph and icon metrics are admitted");
        let colors = font().colorize(marked, [1, 2, 3, 4]).colors;
        assert_eq!(colors.len(), 2, "the colourizer excludes the icon token");
        assert_eq!(
            layout
                .glyphs
                .iter()
                .map(|glyph| glyph.visible_char_index)
                .collect::<Vec<_>>(),
            vec![0, 1],
            "the layout index must address the same icon-free colour vector"
        );
    }

    #[test]
    fn unresolved_inline_icon_refuses_instead_of_disappearing_from_the_stream() {
        let error = layout_engine_text(
            &font(),
            "A\u{00a3}tankB",
            100,
            EngineTextBorder::ExplicitlyAbsent,
        )
        .expect_err("icon has no admitted sprite metrics");
        assert_eq!(
            error,
            EngineTextLayoutError::InlineIconRequiresResolvedMetrics { byte_index: 1 }
        );
    }

    #[test]
    fn authored_inline_icon_frames_are_one_based_like_gamebitmapfont() {
        assert_eq!(engine_inline_icon_frame_index(None), Some(0));
        assert_eq!(engine_inline_icon_frame_index(Some(1)), Some(0));
        assert_eq!(engine_inline_icon_frame_index(Some(3)), Some(2));
        assert_eq!(engine_inline_icon_frame_index(Some(0)), None);
    }

    #[test]
    fn empty_inline_icon_token_is_not_admitted_by_a_placeholder_metric() {
        let metrics = [ResolvedInlineIconMetrics {
            token: String::new(),
            sprite: "GFX_placeholder".into(),
            frame: 0,
            advance: 10,
            height: 10,
        }];
        let error = layout_engine_text_with_icons(
            &font(),
            "A\u{00a3} B",
            100,
            EngineTextBorder::ExplicitlyAbsent,
            &metrics,
        )
        .expect_err("the engine decoder requires a nonempty icon key");
        assert_eq!(
            error,
            EngineTextLayoutError::InlineIconMetricsMismatch {
                ordinal: 0,
                token: String::new(),
            }
        );
    }

    #[test]
    fn authored_inline_icon_frame_suffix_must_match_resolved_frame() {
        let metrics = [ResolvedInlineIconMetrics {
            token: "tank|2".into(),
            sprite: "GFX_text_tank".into(),
            frame: 2,
            advance: 10,
            height: 10,
        }];
        let error = layout_engine_text_with_icons(
            &font(),
            "A\u{00a3}tank|2B",
            100,
            EngineTextBorder::ExplicitlyAbsent,
            &metrics,
        )
        .expect_err("the authored frame suffix cannot resolve to another frame");
        assert_eq!(
            error,
            EngineTextLayoutError::InlineIconMetricsMismatch {
                ordinal: 0,
                token: "tank|2".into(),
            }
        );
    }

    #[test]
    fn bare_inline_icon_frame_separator_refuses_at_the_unproven_source_boundary() {
        let metrics = [ResolvedInlineIconMetrics {
            token: "tank|".into(),
            sprite: "GFX_text_tank".into(),
            frame: 0,
            advance: 10,
            height: 10,
        }];
        let error = layout_engine_text_with_icons(
            &font(),
            "A\u{00a3}tank| B",
            100,
            EngineTextBorder::ExplicitlyAbsent,
            &metrics,
        )
        .expect_err("a bare source-level frame separator has no derived frame identity");
        assert_eq!(
            error,
            EngineTextLayoutError::InlineIconMetricsMismatch {
                ordinal: 0,
                token: "tank|".into(),
            }
        );
    }

    #[test]
    fn extreme_metrics_saturate_without_a_debug_or_release_overflow_panic() {
        let mut extreme = font();
        extreme.chars.get_mut(&('A' as u32)).unwrap().xadvance = i32::MAX;
        let layout =
            layout_engine_text(&extreme, "AA", i32::MAX, EngineTextBorder::ExplicitlyAbsent)
                .expect("saturating layout remains total");
        assert_eq!(layout.glyphs[1].pen_x, 0);
        assert_eq!(layout.content_width, i32::MAX);
    }
}
