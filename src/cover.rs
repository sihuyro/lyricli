use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use image::{RgbImage, imageops::FilterType};

use crate::cli::CoverStyle;
use crate::lyrics::cache_dir;
use crate::player::sanitize_filename;

const STANDARD_RAMP: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];
const DETAILED_RAMP: &[char] = &[
    ' ', '.', '\'', '`', '^', '"', ',', ':', ';', 'I', 'l', '!', 'i', '>', '<', '~', '+', '_',
    '-', '?', ']', '[', '}', '{', '1', ')', '(', '|', '\\', '/', 't', 'f', 'j', 'r', 'x', 'n',
    'u', 'v', 'c', 'z', 'X', 'Y', 'U', 'J', 'C', 'L', 'Q', '0', 'O', 'Z', 'm', 'w', 'q', 'p',
    'd', 'b', 'k', 'h', 'a', 'o', '*', '#', 'M', 'W', '&', '8', '%', 'B', '@', '$',
];
const BLOCKS_RAMP: &[char] = &[' ', '░', '▒', '▓', '█'];

const BRAILLE_MAP: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
const BAYER_2X4: [[u8; 2]; 4] = [[20, 136], [194, 78], [50, 165], [224, 107]];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    pub accent: [u8; 3],
    pub sung: [u8; 3],
}

impl Default for ThemePalette {
    fn default() -> Self {
        Self {
            accent: [242, 109, 111],
            sung: [250, 250, 255],
        }
    }
}

#[derive(Clone, Debug)]
pub struct AsciiCover {
    pub width: u32,
    pub height: u32,
    pub rows: Vec<String>,
    pub palette: ThemePalette,
}

pub fn load_and_render_cover(
    art_url: &str,
    cols: u32,
    style: CoverStyle,
) -> Option<AsciiCover> {
    let img = load_cover_image(art_url)?;
    Some(render_cover_image(&img, cols, style))
}

fn load_cover_image(art_url: &str) -> Option<RgbImage> {
    if let Some(local_path) = art_url.strip_prefix("file://") {
        return image::open(Path::new(local_path)).ok().map(|i| i.to_rgb8());
    }

    if art_url.starts_with("http://") || art_url.starts_with("https://") {
        let key = sanitize_filename(art_url);
        let cache_path = cache_dir().join("covers").join(format!("{key}.img"));

        if !cache_path.exists() {
            let status = Command::new("curl")
                .args(["-sSL", "--max-time", "6", "-o"])
                .arg(&cache_path)
                .arg(art_url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()?;
            if !status.success() {
                return None;
            }
        }

        let bytes = fs::read(&cache_path).ok()?;
        return image::load_from_memory(&bytes).ok().map(|i| i.to_rgb8());
    }

    None
}

pub fn render_cover_image(img: &RgbImage, cols: u32, style: CoverStyle) -> AsciiCover {
    let grid_w = cols.clamp(12, 80);
    // Album covers are 1:1 square; terminal cells are roughly 1:2 width-to-height.
    let grid_h = (grid_w / 2).max(6);
    let palette = extract_theme_palette(img);

    let rows = match style {
        CoverStyle::Halfblock => render_halfblock(img, grid_w, grid_h),
        CoverStyle::Braille => render_braille(img, grid_w, grid_h),
        CoverStyle::Blocks => render_ramp(img, grid_w, grid_h, BLOCKS_RAMP),
        CoverStyle::Detailed => render_ramp(img, grid_w, grid_h, DETAILED_RAMP),
        CoverStyle::Standard => render_ramp(img, grid_w, grid_h, STANDARD_RAMP),
    };

    AsciiCover {
        width: grid_w,
        height: grid_h,
        rows,
        palette,
    }
}

pub fn extract_theme_palette(img: &RgbImage) -> ThemePalette {
    let small = image::imageops::resize(img, 24, 24, FilterType::Triangle);
    let mut buckets = [(0u32, 0u32, 0u32, 0u32); 512];

    for px in small.pixels() {
        let [r, g, b] = px.0;
        let max_c = r.max(g).max(b);
        if max_c < 30 {
            continue;
        }
        let idx = (((r as usize) >> 5) << 6) | (((g as usize) >> 5) << 3) | ((b as usize) >> 5);
        buckets[idx].0 += 1;
        buckets[idx].1 += r as u32;
        buckets[idx].2 += g as u32;
        buckets[idx].3 += b as u32;
    }

    let mut best_score = 0u64;
    let mut best_rgb: Option<[u8; 3]> = None;

    for &(count, sr, sg, sb) in &buckets {
        if count == 0 {
            continue;
        }
        let r = (sr / count) as u8;
        let g = (sg / count) as u8;
        let b = (sb / count) as u8;
        let max_c = r.max(g).max(b) as u64;
        let min_c = r.min(g).min(b) as u64;
        let chroma = max_c - min_c;
        let score = (count as u64) * (12 + chroma * 4 + max_c / 4);
        if score > best_score {
            best_score = score;
            best_rgb = Some([r, g, b]);
        }
    }

    let Some(raw) = best_rgb else {
        return ThemePalette::default();
    };
    let max_c = raw[0].max(raw[1]).max(raw[2]) as u32;
    let min_c = raw[0].min(raw[1]).min(raw[2]) as u32;
    if max_c - min_c < 18 {
        return ThemePalette {
            accent: [175, 200, 240],
            sung: [250, 250, 255],
        };
    }

    let target = max_c.clamp(195, 248);
    let accent = [
        ((raw[0] as u32 * target) / max_c).min(255) as u8,
        ((raw[1] as u32 * target) / max_c).min(255) as u8,
        ((raw[2] as u32 * target) / max_c).min(255) as u8,
    ];
    let sung = [
        ((255u32 * 72 + accent[0] as u32 * 28) / 100) as u8,
        ((255u32 * 72 + accent[1] as u32 * 28) / 100) as u8,
        ((255u32 * 72 + accent[2] as u32 * 28) / 100) as u8,
    ];
    ThemePalette { accent, sung }
}

fn render_halfblock(img: &RgbImage, grid_w: u32, grid_h: u32) -> Vec<String> {
    let sample_w = grid_w;
    let sample_h = grid_h * 2;
    let resized = image::imageops::resize(img, sample_w, sample_h, FilterType::Lanczos3);
    let raw = resized.as_raw();

    let mut rows = Vec::with_capacity(grid_h as usize);
    for cy in 0..grid_h {
        let mut line = String::with_capacity((grid_w * 25) as usize);
        let mut last_fg: Option<[u8; 3]> = None;
        let mut last_bg: Option<[u8; 3]> = None;

        let top_y = cy * 2;
        let bot_y = top_y + 1;

        for cx in 0..grid_w {
            let top_idx = ((top_y * sample_w + cx) * 3) as usize;
            let bot_idx = ((bot_y * sample_w + cx) * 3) as usize;

            let fg = [raw[top_idx], raw[top_idx + 1], raw[top_idx + 2]];
            let bg = [raw[bot_idx], raw[bot_idx + 1], raw[bot_idx + 2]];

            if last_fg != Some(fg) {
                let _ = write!(line, "\x1b[38;2;{};{};{}m", fg[0], fg[1], fg[2]);
                last_fg = Some(fg);
            }
            if last_bg != Some(bg) {
                let _ = write!(line, "\x1b[48;2;{};{};{}m", bg[0], bg[1], bg[2]);
                last_bg = Some(bg);
            }
            line.push('▀');
        }
        line.push_str("\x1b[0m");
        rows.push(line);
    }

    rows
}

fn render_braille(img: &RgbImage, grid_w: u32, grid_h: u32) -> Vec<String> {
    let sample_w = grid_w * 2;
    let sample_h = grid_h * 4;
    let resized = image::imageops::resize(img, sample_w, sample_h, FilterType::Lanczos3);
    let raw = resized.as_raw();

    let mut rows = Vec::with_capacity(grid_h as usize);
    for cy in 0..grid_h {
        let mut line = String::with_capacity((grid_w * 20) as usize);
        let mut last_rgb: Option<[u8; 3]> = None;

        for cx in 0..grid_w {
            let mut mask = 0u8;
            let mut sum_r = 0u32;
            let mut sum_g = 0u32;
            let mut sum_b = 0u32;
            let mut count = 0u32;

            for dy in 0..4 {
                for dx in 0..2 {
                    let px = cx * 2 + dx;
                    let py = cy * 4 + dy;
                    let idx = ((py * sample_w + px) * 3) as usize;
                    let r = raw[idx];
                    let g = raw[idx + 1];
                    let b = raw[idx + 2];
                    let luma = boosted_luma(r, g, b);
                    if luma >= BAYER_2X4[dy as usize][dx as usize] {
                        mask |= BRAILLE_MAP[dy as usize][dx as usize];
                        sum_r += r as u32;
                        sum_g += g as u32;
                        sum_b += b as u32;
                        count += 1;
                    }
                }
            }

            if mask == 0 {
                line.push(' ');
                continue;
            }

            let ch = char::from_u32(0x2800 + mask as u32).unwrap_or(' ');
            let rgb = boost_rgb(
                (sum_r / count.max(1)) as u8,
                (sum_g / count.max(1)) as u8,
                (sum_b / count.max(1)) as u8,
            );
            if last_rgb != Some(rgb) {
                let _ = write!(line, "\x1b[38;2;{};{};{}m", rgb[0], rgb[1], rgb[2]);
                last_rgb = Some(rgb);
            }
            line.push(ch);
        }
        line.push_str("\x1b[0m");
        rows.push(line);
    }

    rows
}

fn render_ramp(img: &RgbImage, grid_w: u32, grid_h: u32, ramp: &[char]) -> Vec<String> {
    let resized = image::imageops::resize(img, grid_w, grid_h, FilterType::Lanczos3);
    let raw = resized.as_raw();
    let max_idx = ramp.len().saturating_sub(1);

    let mut rows = Vec::with_capacity(grid_h as usize);
    for y in 0..grid_h {
        let mut line = String::with_capacity((grid_w * 20) as usize);
        let mut last_rgb: Option<[u8; 3]> = None;

        for x in 0..grid_w {
            let idx = ((y * grid_w + x) * 3) as usize;
            let r = raw[idx];
            let g = raw[idx + 1];
            let b = raw[idx + 2];
            let luma = boosted_luma(r, g, b);
            let char_idx = ((luma as usize) * max_idx) / 255;
            let ch = ramp[char_idx];
            if ch == ' ' {
                line.push(' ');
                continue;
            }
            let rgb = boost_rgb(r, g, b);
            if last_rgb != Some(rgb) {
                let _ = write!(line, "\x1b[38;2;{};{};{}m", rgb[0], rgb[1], rgb[2]);
                last_rgb = Some(rgb);
            }
            line.push(ch);
        }
        line.push_str("\x1b[0m");
        rows.push(line);
    }

    rows
}

fn boosted_luma(r: u8, g: u8, b: u8) -> u8 {
    let raw = ((77 * (r as u32) + 150 * (g as u32) + 29 * (b as u32)) >> 8) as f32;
    ((raw / 255.0).powf(0.78) * 255.0).round().clamp(0.0, 255.0) as u8
}

fn boost_rgb(r: u8, g: u8, b: u8) -> [u8; 3] {
    let max_c = r.max(g).max(b) as u32;
    if max_c == 0 {
        return [180, 180, 180];
    }
    let target = 115 + (max_c * 140) / 255;
    [
        ((r as u32 * target) / max_c).min(255) as u8,
        ((g as u32 * target) / max_c).min(255) as u8,
        ((b as u32 * target) / max_c).min(255) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn renders_cover_in_all_styles_with_expected_dimensions() {
        let img = RgbImage::from_pixel(64, 64, Rgb([200, 60, 80]));
        for style in [
            CoverStyle::Halfblock,
            CoverStyle::Braille,
            CoverStyle::Blocks,
            CoverStyle::Detailed,
            CoverStyle::Standard,
        ] {
            let cov = render_cover_image(&img, 34, style);
            assert_eq!(cov.width, 34);
            assert_eq!(cov.height, 17);
            assert_eq!(cov.rows.len(), 17);
        }
    }
}

