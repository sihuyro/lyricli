use crate::cli::LyricArtStyle;

#[derive(Clone, Debug)]
pub struct RenderedAsciiLyric {
    pub rows: Vec<String>,
    pub visual_widths: Vec<usize>,
}

pub fn has_unsupported_glyphs(text: &str) -> bool {
    let mut alpha_total = 0usize;
    let mut non_ascii = 0usize;
    for ch in text.chars() {
        if ch.is_alphabetic() {
            alpha_total += 1;
            if !ch.is_ascii_alphabetic() {
                non_ascii += 1;
            }
        }
    }
    alpha_total > 0 && non_ascii * 3 >= alpha_total
}

pub fn render_ascii_lyric(
    text: &str,
    max_cols: usize,
    progress: f64,
    style: LyricArtStyle,
) -> RenderedAsciiLyric {
    let clean = text.trim();
    if clean.is_empty() {
        return RenderedAsciiLyric {
            rows: Vec::new(),
            visual_widths: Vec::new(),
        };
    }

    let char_stride = match style {
        LyricArtStyle::Braille => 3,
        _ => 5,
    };
    let max_chars_per_line = (max_cols / char_stride).max(6);
    let wrapped_lines = wrap_words(clean, max_chars_per_line, 4);

    let total_chars: usize = wrapped_lines.iter().map(|l| l.chars().count()).sum();
    let sung_chars_float = (total_chars as f64) * progress.clamp(0.0, 1.0);

    let mut out_rows = Vec::new();
    let mut out_widths = Vec::new();
    let mut chars_before_line = 0usize;

    for (line_idx, line_str) in wrapped_lines.iter().enumerate() {
        if line_idx > 0 {
            out_rows.push(String::new());
            out_widths.push(0);
        }

        let line_char_count = line_str.chars().count();
        let line_progress = if line_char_count == 0 {
            1.0
        } else {
            ((sung_chars_float - chars_before_line as f64) / line_char_count as f64).clamp(0.0, 1.0)
        };
        chars_before_line += line_char_count;

        let (mut rows, width) = match style {
            LyricArtStyle::Block => render_line_halfblock(line_str, line_progress),
            LyricArtStyle::Banner => render_line_banner(line_str, line_progress),
            LyricArtStyle::Braille => render_line_braille(line_str, line_progress),
        };

        for _ in 0..rows.len() {
            out_widths.push(width);
        }
        out_rows.append(&mut rows);
    }

    RenderedAsciiLyric {
        rows: out_rows,
        visual_widths: out_widths,
    }
}

fn wrap_words(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        let cur_len = current.chars().count();

        if cur_len == 0 {
            if word_len <= max_chars {
                current.push_str(word);
            } else {
                let truncated: String = word.chars().take(max_chars).collect();
                current.push_str(&truncated);
            }
        } else if cur_len + 1 + word_len <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            if lines.len() >= max_lines {
                return lines;
            }
            current = if word_len <= max_chars {
                word.to_string()
            } else {
                word.chars().take(max_chars).collect()
            };
        }
    }

    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    }

    lines
}

fn render_line_halfblock(line: &str, progress: f64) -> (Vec<String>, usize) {
    let pixel_cols = build_bitmap_columns(line);
    let total_w = pixel_cols.len();
    let sung_col = ((total_w as f64) * progress).round() as usize;
    let edge_col = sung_col.saturating_sub(1);

    let mut rows = vec![String::with_capacity(total_w * 8); 3];

    for (row_idx, row_buf) in rows.iter_mut().enumerate() {
        let py_top = row_idx * 2;
        let py_bot = py_top + 1;
        let mut current_zone: Option<u8> = None;

        for (x, col_bits) in pixel_cols.iter().enumerate() {
            let top_on = (col_bits >> (5 - py_top)) & 1 == 1;
            let bot_on = (col_bits >> (5 - py_bot)) & 1 == 1;

            let ch = match (top_on, bot_on) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };

            if ch == ' ' {
                row_buf.push(' ');
                continue;
            }

            let zone = if x < edge_col {
                0
            } else if x <= sung_col && progress > 0.0 && progress < 1.0 {
                1
            } else if progress >= 1.0 {
                0
            } else {
                2
            };

            if current_zone != Some(zone) {
                match zone {
                    0 => row_buf.push_str("\x1b[1;38;2;250;250;255m"),
                    1 => row_buf.push_str("\x1b[1;38;2;242;109;111m"),
                    _ => row_buf.push_str("\x1b[38;2;112;112;130m"),
                }
                current_zone = Some(zone);
            }
            row_buf.push(ch);
        }
        row_buf.push_str("\x1b[0m");
    }

    (rows, total_w)
}

fn render_line_banner(line: &str, progress: f64) -> (Vec<String>, usize) {
    let pixel_cols = build_bitmap_columns(line);
    let total_w = pixel_cols.len();
    let sung_col = ((total_w as f64) * progress).round() as usize;

    let mut rows = vec![String::with_capacity(total_w * 8); 5];

    for (py, row_buf) in rows.iter_mut().enumerate() {
        let mut current_sung: Option<bool> = None;
        for (x, col_bits) in pixel_cols.iter().enumerate() {
            let on = (col_bits >> (5 - py)) & 1 == 1;
            if !on {
                row_buf.push(' ');
                continue;
            }
            let is_sung = progress >= 1.0 || (progress > 0.0 && x < sung_col);
            if current_sung != Some(is_sung) {
                if is_sung {
                    row_buf.push_str("\x1b[1;38;2;242;109;111m");
                } else {
                    row_buf.push_str("\x1b[38;2;112;112;130m");
                }
                current_sung = Some(is_sung);
            }
            let ch = if is_sung { '#' } else { ':' };
            row_buf.push(ch);
        }
        row_buf.push_str("\x1b[0m");
    }

    (rows, total_w)
}

fn render_line_braille(line: &str, progress: f64) -> (Vec<String>, usize) {
    let pixel_cols = build_bitmap_columns(line);
    let total_px_w = pixel_cols.len();
    let cell_w = total_px_w.div_ceil(2);
    let sung_cell = ((cell_w as f64) * progress).round() as usize;

    let braille_dots: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let mut rows = vec![String::with_capacity(cell_w * 8); 2];

    for (cell_y, row_buf) in rows.iter_mut().enumerate() {
        let mut current_sung: Option<bool> = None;
        for cx in 0..cell_w {
            let mut mask = 0u8;
            for (dy, dot_row) in braille_dots.iter().enumerate() {
                let py = cell_y * 3 + dy;
                if py >= 6 {
                    continue;
                }
                for (dx, dot_bit) in dot_row.iter().enumerate() {
                    let px = cx * 2 + dx;
                    if px < total_px_w {
                        let on = (pixel_cols[px] >> (5 - py)) & 1 == 1;
                        if on {
                            mask |= dot_bit;
                        }
                    }
                }
            }
            if mask == 0 {
                row_buf.push(' ');
                continue;
            }
            let is_sung = progress >= 1.0 || (progress > 0.0 && cx < sung_cell);
            if current_sung != Some(is_sung) {
                if is_sung {
                    row_buf.push_str("\x1b[1;38;2;250;250;255m");
                } else {
                    row_buf.push_str("\x1b[38;2;112;112;130m");
                }
                current_sung = Some(is_sung);
            }
            let ch = char::from_u32(0x2800 + mask as u32).unwrap_or(' ');
            row_buf.push(ch);
        }
        row_buf.push_str("\x1b[0m");
    }

    (rows, cell_w)
}

fn build_bitmap_columns(line: &str) -> Vec<u8> {
    let mut cols = Vec::with_capacity(line.chars().count() * 5);
    for (i, ch) in line.chars().enumerate() {
        if i > 0 {
            cols.push(0);
        }
        let glyph = glyph_4x6(ch);
        cols.extend_from_slice(&glyph);
    }
    cols
}

// Each glyph is 4 columns wide; each column byte holds 6 vertical bits (bit 5 = top row, bit 0 = bottom row).
fn glyph_4x6(ch: char) -> [u8; 4] {
    match ch.to_ascii_uppercase() {
        'A' => [0b011110, 0b100100, 0b100100, 0b011110],
        'B' => [0b111110, 0b101010, 0b101010, 0b010100],
        'C' => [0b011100, 0b100010, 0b100010, 0b010100],
        'D' => [0b111110, 0b100010, 0b100010, 0b011100],
        'E' => [0b111110, 0b101010, 0b101010, 0b100010],
        'F' => [0b111110, 0b101000, 0b101000, 0b100000],
        'G' => [0b011100, 0b100010, 0b101010, 0b011100],
        'H' => [0b111110, 0b001000, 0b001000, 0b111110],
        'I' => [0b100010, 0b111110, 0b100010, 0b000000],
        'J' => [0b000100, 0b000010, 0b100010, 0b011100],
        'K' => [0b111110, 0b001000, 0b010100, 0b100010],
        'L' => [0b111110, 0b000010, 0b000010, 0b000010],
        'M' => [0b111110, 0b010000, 0b010000, 0b111110],
        'N' => [0b111110, 0b010000, 0b001100, 0b111110],
        'O' => [0b011100, 0b100010, 0b100010, 0b011100],
        'P' => [0b111110, 0b100100, 0b100100, 0b011000],
        'Q' => [0b011100, 0b100010, 0b100110, 0b011101],
        'R' => [0b111110, 0b100100, 0b101100, 0b010010],
        'S' => [0b010010, 0b101010, 0b101010, 0b010100],
        'T' => [0b100000, 0b111110, 0b100000, 0b100000],
        'U' => [0b111100, 0b000010, 0b000010, 0b111100],
        'V' => [0b111000, 0b000110, 0b000110, 0b111000],
        'W' => [0b111110, 0b000100, 0b000100, 0b111110],
        'X' => [0b110010, 0b001100, 0b001100, 0b110010],
        'Y' => [0b110000, 0b001110, 0b001110, 0b110000],
        'Z' => [0b100010, 0b100110, 0b101010, 0b110010],
        '0' => [0b011100, 0b100110, 0b110010, 0b011100],
        '1' => [0b010010, 0b111110, 0b000010, 0b000000],
        '2' => [0b010010, 0b100110, 0b101010, 0b010010],
        '3' => [0b100010, 0b101010, 0b101010, 0b010100],
        '4' => [0b001100, 0b010100, 0b111110, 0b000100],
        '5' => [0b111010, 0b101010, 0b101010, 0b100100],
        '6' => [0b011100, 0b101010, 0b101010, 0b000100],
        '7' => [0b100000, 0b100110, 0b101000, 0b110000],
        '8' => [0b010100, 0b101010, 0b101010, 0b010100],
        '9' => [0b010000, 0b101010, 0b101010, 0b011100],
        '\'' | '’' | '`' => [0b000000, 0b110000, 0b000000, 0b000000],
        '"' | '“' | '”' => [0b110000, 0b000000, 0b110000, 0b000000],
        ',' => [0b000000, 0b000011, 0b000010, 0b000000],
        '.' => [0b000000, 0b000010, 0b000000, 0b000000],
        '!' => [0b000000, 0b111010, 0b000000, 0b000000],
        '?' => [0b010000, 0b100110, 0b101000, 0b010000],
        '-' | '\u{2014}' | '\u{2013}' => [0b001000, 0b001000, 0b001000, 0b000000],
        ':' => [0b000000, 0b010010, 0b000000, 0b000000],
        '(' | '（' => [0b000000, 0b011100, 0b100010, 0b000000],
        ')' | '）' => [0b000000, 0b100010, 0b011100, 0b000000],
        '$' => [0b010010, 0b111111, 0b101010, 0b010100],
        '&' => [0b010100, 0b101010, 0b010100, 0b001010],
        '/' => [0b000010, 0b000100, 0b001000, 0b010000],
        '+' => [0b001000, 0b011100, 0b001000, 0b000000],
        '*' => [0b010100, 0b001000, 0b010100, 0b000000],
        '♪' => [0b000110, 0b011110, 0b010000, 0b011100],
        ' ' => [0b000000, 0b000000, 0b000000, 0b000000],
        _ => [0b000000, 0b001100, 0b001100, 0b000000],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_ascii_lyric_with_karaoke_progress() {
        for style in [
            LyricArtStyle::Block,
            LyricArtStyle::Banner,
            LyricArtStyle::Braille,
        ] {
            let rendered = render_ascii_lyric("Hello World", 80, 0.5, style);
            assert!(!rendered.rows.is_empty());
            assert_eq!(rendered.rows.len(), rendered.visual_widths.len());
            assert!(rendered.visual_widths[0] > 0);
        }
    }
}
