use std::fmt::Write as FmtWrite;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};

use crate::ascii_font::{has_unsupported_glyphs, render_ascii_lyric};
use crate::cli::{CoverStyle, LyricArtStyle};
use crate::cover::{AsciiCover, ThemePalette};
use crate::lyrics::LyricsPayload;
use crate::player::{PlaybackStatus, TrackState};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UserAction {
    None,
    PlayPause,
    NextTrack,
    PrevTrack,
    SeekForward,
    SeekBackward,
    ToggleCover,
    CycleCoverStyle,
    CycleLyricStyle,
    ToggleTextLyrics,
    ToggleRomanize,
    ToggleDynamicTheme,
    RefreshLyrics,
    ToggleHud,
    OffsetEarlier,
    OffsetLater,
    ScrollUp,
    ScrollDown,
    Quit,
}

pub fn poll_user_action(timeout: Duration) -> Result<UserAction> {
    if !event::poll(timeout)? {
        return Ok(UserAction::None);
    }
    if let Event::Key(key) = event::read()? {
        if key.kind != KeyEventKind::Press {
            return Ok(UserAction::None);
        }
        let action = match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => UserAction::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => UserAction::Quit,
            KeyCode::Char(' ') => UserAction::PlayPause,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('>') => UserAction::NextTrack,
            KeyCode::Char('p') | KeyCode::Char('P') | KeyCode::Char('<') => UserAction::PrevTrack,
            KeyCode::Right | KeyCode::Char('l') => UserAction::SeekForward,
            KeyCode::Left => UserAction::SeekBackward,
            KeyCode::Char('c') | KeyCode::Char('C') => UserAction::ToggleCover,
            KeyCode::Char('v') | KeyCode::Char('V') => UserAction::CycleCoverStyle,
            KeyCode::Char('f') | KeyCode::Char('F') => UserAction::CycleLyricStyle,
            KeyCode::Char('t') | KeyCode::Char('T') => UserAction::ToggleTextLyrics,
            KeyCode::Char('r') => UserAction::ToggleRomanize,
            KeyCode::Char('R') | KeyCode::F(5) => UserAction::RefreshLyrics,
            KeyCode::Char('d') | KeyCode::Char('D') => UserAction::ToggleDynamicTheme,
            KeyCode::Char('h') | KeyCode::Char('H') => UserAction::ToggleHud,
            KeyCode::Char('[') | KeyCode::Char('-') => UserAction::OffsetEarlier,
            KeyCode::Char(']') | KeyCode::Char('=') | KeyCode::Char('+') => UserAction::OffsetLater,
            KeyCode::Up | KeyCode::Char('k') => UserAction::ScrollUp,
            KeyCode::Down | KeyCode::Char('j') => UserAction::ScrollDown,
            _ => UserAction::None,
        };
        return Ok(action);
    }
    Ok(UserAction::None)
}

pub struct TerminalUi {
    buf: String,
    start_time: Instant,
}

impl TerminalUi {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All))?;
        Ok(Self {
            buf: String::with_capacity(65536),
            start_time: Instant::now(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_waiting(
        &mut self,
        hide_hud: bool,
        cover_style: CoverStyle,
        lyric_style: LyricArtStyle,
        show_text_lyrics: bool,
        romanize: bool,
        dynamic_color: bool,
    ) -> Result<()> {
        let (term_w, term_h) = terminal_size();
        self.buf.clear();
        self.buf.push_str("\x1b[2J");

        let palette = ThemePalette::default();
        let twinkle_t = self.start_time.elapsed().as_secs_f64();
        let usable_h = if hide_hud {
            term_h as isize
        } else {
            (term_h.saturating_sub(1)) as isize
        };
        let center_y = (usable_h / 2).max(1);
        let mut occupied = Vec::new();
        draw_ascii_active(
            &mut self.buf,
            "WAITING FOR SPOTIFY",
            1,
            term_w as usize,
            center_y,
            1,
            usable_h,
            1.0,
            lyric_style,
            palette,
            &mut occupied,
        );
        draw_starfield(
            &mut self.buf,
            1,
            term_w as usize,
            1,
            usable_h,
            &occupied,
            palette,
            twinkle_t,
        );

        if !hide_hud {
            draw_hud_bar(
                &mut self.buf,
                term_w,
                term_h,
                cover_style,
                lyric_style,
                show_text_lyrics,
                romanize,
                dynamic_color,
                0,
            );
        }

        self.flush()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_track(
        &mut self,
        track: &TrackState,
        pos_secs: f64,
        lyrics: &LyricsPayload,
        cover: Option<&AsciiCover>,
        show_cover: bool,
        cover_style: CoverStyle,
        lyric_style: LyricArtStyle,
        show_text_lyrics: bool,
        romanize: bool,
        dynamic_color: bool,
        hide_hud: bool,
        offset_ms: i64,
        manual_scroll: isize,
        loading_lyrics: bool,
    ) -> Result<()> {
        let (term_w, term_h) = terminal_size();
        self.buf.clear();
        self.buf.push_str("\x1b[2J");

        let adjusted_pos = (pos_secs - (offset_ms as f64 / 1000.0)).max(0.0);
        let palette = if dynamic_color {
            cover.map(|c| c.palette).unwrap_or_default()
        } else {
            ThemePalette::default()
        };
        let twinkle_t = self.start_time.elapsed().as_secs_f64();

        let can_show_split = show_cover
            && cover.is_some()
            && term_w >= cover.map(|c| c.width).unwrap_or(34) + 46
            && term_h >= 16;

        if can_show_split {
            let cov = cover.expect("checked some");
            self.render_split_centered(
                term_w,
                term_h,
                track,
                pos_secs,
                adjusted_pos,
                lyrics,
                cov,
                lyric_style,
                show_text_lyrics,
                romanize,
                palette,
                twinkle_t,
                manual_scroll,
                loading_lyrics,
                hide_hud,
            );
        } else {
            self.render_pure_centered(
                term_w,
                term_h,
                track,
                pos_secs,
                adjusted_pos,
                lyrics,
                lyric_style,
                show_text_lyrics,
                romanize,
                palette,
                twinkle_t,
                manual_scroll,
                loading_lyrics,
                hide_hud,
            );
        }

        if !hide_hud {
            draw_hud_bar(
                &mut self.buf,
                term_w,
                term_h,
                cover_style,
                lyric_style,
                show_text_lyrics,
                romanize,
                dynamic_color,
                offset_ms,
            );
        }

        self.flush()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_split_centered(
        &mut self,
        term_w: u32,
        term_h: u32,
        track: &TrackState,
        pos_secs: f64,
        adjusted_pos: f64,
        lyrics: &LyricsPayload,
        cover: &AsciiCover,
        lyric_style: LyricArtStyle,
        show_text_lyrics: bool,
        romanize: bool,
        palette: ThemePalette,
        twinkle_t: f64,
        manual_scroll: isize,
        loading_lyrics: bool,
        hide_hud: bool,
    ) {
        let cov_w = cover.width as usize;
        let gap = 6usize;
        let max_lyrics_w = (term_w as usize).saturating_sub(cov_w + gap + 6).min(88);
        let total_w = cov_w + gap + max_lyrics_w;
        let left_x = ((term_w as usize).saturating_sub(total_w) / 2) + 1;
        let right_x = left_x + cov_w + gap;

        let usable_h = if hide_hud {
            term_h
        } else {
            term_h.saturating_sub(1)
        };
        let center_y = (usable_h / 2).max(1);

        let left_block_h = cover.height + 4;
        let left_top_y = center_y.saturating_sub(left_block_h / 2).max(1);

        for (i, row_str) in cover.rows.iter().enumerate() {
            let y = left_top_y + (i as u32);
            if y <= usable_h {
                let _ = write!(self.buf, "\x1b[{y};{left_x}H{row_str}");
            }
        }

        let [ar, ag, ab] = palette.accent;
        let meta_y = left_top_y + cover.height + 1;
        if meta_y <= usable_h {
            let title_trunc = truncate_chars(&track.title, cov_w);
            let title_pad = cov_w.saturating_sub(display_width(&title_trunc)) / 2;
            let _ = write!(
                self.buf,
                "\x1b[{meta_y};{}H\x1b[1;38;2;240;240;245m{}\x1b[0m",
                left_x + title_pad,
                title_trunc
            );
        }
        if meta_y < usable_h {
            let artist_album = if track.album.is_empty() {
                track.artist.clone()
            } else {
                format!("{} · {}", track.artist, track.album)
            };
            let sub_trunc = truncate_chars(&artist_album, cov_w);
            let sub_pad = cov_w.saturating_sub(display_width(&sub_trunc)) / 2;
            let _ = write!(
                self.buf,
                "\x1b[{};{}H\x1b[38;2;{ar};{ag};{ab}m{}\x1b[0m",
                meta_y + 1,
                left_x + sub_pad,
                sub_trunc
            );
        }
        if meta_y + 2 <= usable_h {
            let prog =
                format_progress_bar(pos_secs, track.duration_secs, cov_w, track.status, palette);
            let _ = write!(self.buf, "\x1b[{};{}H{}", meta_y + 2, left_x, prog);
        }

        draw_lyrics_column(
            &mut self.buf,
            right_x,
            max_lyrics_w,
            center_y as isize,
            1,
            usable_h as isize,
            adjusted_pos,
            track.duration_secs,
            lyrics,
            lyric_style,
            show_text_lyrics,
            romanize,
            palette,
            twinkle_t,
            manual_scroll,
            loading_lyrics,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn render_pure_centered(
        &mut self,
        term_w: u32,
        term_h: u32,
        track: &TrackState,
        pos_secs: f64,
        adjusted_pos: f64,
        lyrics: &LyricsPayload,
        lyric_style: LyricArtStyle,
        show_text_lyrics: bool,
        romanize: bool,
        palette: ThemePalette,
        twinkle_t: f64,
        manual_scroll: isize,
        loading_lyrics: bool,
        hide_hud: bool,
    ) {
        let usable_h = if hide_hud {
            term_h
        } else {
            term_h.saturating_sub(1)
        };
        let max_w = (term_w as usize).saturating_sub(6).max(20);
        let [ar, ag, ab] = palette.accent;
        let header_style = format!("\x1b[1;38;2;{ar};{ag};{ab}m");

        let header_line = format!("{}  ·  {}", track.title, track.artist);
        let header_trunc = truncate_chars(&header_line, max_w);
        draw_centered_text(&mut self.buf, 2, term_w, &header_trunc, &header_style);

        let bar_w = max_w.min(48);
        let prog =
            format_progress_bar(pos_secs, track.duration_secs, bar_w, track.status, palette);
        let prog_col = ((term_w as usize).saturating_sub(bar_w) / 2) + 1;
        if usable_h >= 4 {
            let _ = write!(self.buf, "\x1b[3;{prog_col}H{prog}");
        }

        let stage_top = 5isize;
        let stage_bottom = usable_h as isize;
        let center_y = ((stage_top + stage_bottom) / 2).max(stage_top);

        draw_lyrics_column(
            &mut self.buf,
            1,
            term_w as usize,
            center_y,
            stage_top,
            stage_bottom,
            adjusted_pos,
            track.duration_secs,
            lyrics,
            lyric_style,
            show_text_lyrics,
            romanize,
            palette,
            twinkle_t,
            manual_scroll,
            loading_lyrics,
        );
    }

    fn flush(&mut self) -> Result<()> {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(self.buf.as_bytes())?;
        handle.flush()?;
        Ok(())
    }
}

impl Drop for TerminalUi {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, MoveTo(0, 0), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_lyrics_column(
    buf: &mut String,
    col_start: usize,
    col_width: usize,
    center_y: isize,
    min_y: isize,
    max_y: isize,
    pos_secs: f64,
    duration_secs: f64,
    lyrics: &LyricsPayload,
    lyric_style: LyricArtStyle,
    show_text_lyrics: bool,
    romanize: bool,
    palette: ThemePalette,
    twinkle_t: f64,
    manual_scroll: isize,
    loading_lyrics: bool,
) {
    let mut occupied: Vec<(isize, usize, usize)> = Vec::new();

    if loading_lyrics {
        draw_ascii_active(
            buf,
            "LOADING LYRICS",
            col_start,
            col_width,
            center_y,
            min_y,
            max_y,
            0.5,
            lyric_style,
            palette,
            &mut occupied,
        );
        draw_starfield(
            buf, col_start, col_width, min_y, max_y, &occupied, palette, twinkle_t,
        );
        return;
    }

    match lyrics {
        LyricsPayload::Instrumental => {
            draw_ascii_active(
                buf,
                "INSTRUMENTAL",
                col_start,
                col_width,
                center_y,
                min_y,
                max_y,
                1.0,
                lyric_style,
                palette,
                &mut occupied,
            );
        }
        LyricsPayload::NotFound => {
            draw_ascii_active(
                buf,
                "NO LYRICS",
                col_start,
                col_width,
                center_y,
                min_y,
                max_y,
                0.0,
                lyric_style,
                palette,
                &mut occupied,
            );
        }
        LyricsPayload::Synced(lines) => {
            let (raw_active, progress) = lyrics.active_index_and_progress(pos_secs, duration_secs);
            let active_idx: isize = match raw_active {
                Some(idx) => (idx as isize + manual_scroll).clamp(0, lines.len() as isize - 1),
                None => -1,
            };

            let (banner_top, banner_bot) = if active_idx == -1 {
                let first_time = lines.first().and_then(|l| l.time_secs).unwrap_or(0.0);
                let rem = (first_time - pos_secs).ceil().max(0.0) as u64;
                let intro_msg = format!("INTRO {rem}S");
                let intro_prog = if first_time > 0.2 {
                    (pos_secs / first_time).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                draw_ascii_active(
                    buf,
                    &intro_msg,
                    col_start,
                    col_width,
                    center_y,
                    min_y,
                    max_y,
                    intro_prog,
                    lyric_style,
                    palette,
                    &mut occupied,
                )
            } else {
                let active_line = &lines[active_idx as usize];
                let effective_prog = if manual_scroll == 0 { progress } else { 1.0 };
                let roma_text = if romanize {
                    active_line
                        .roma
                        .as_deref()
                        .map(str::trim)
                        .filter(|r| !r.is_empty())
                } else {
                    None
                };

                if let Some(roma) = roma_text {
                    let (top_y, bot_y) = draw_ascii_active(
                        buf,
                        roma,
                        col_start,
                        col_width,
                        center_y,
                        min_y,
                        max_y,
                        effective_prog,
                        lyric_style,
                        palette,
                        &mut occupied,
                    );
                    let sub_y = bot_y + 1;
                    if sub_y >= min_y && sub_y <= max_y && active_line.text != roma {
                        let [sr, sg, sb] = palette.sung;
                        let sub_style = format!("\x1b[38;2;{sr};{sg};{sb}m");
                        let native =
                            truncate_chars(&active_line.text, col_width.saturating_sub(4));
                        draw_text_in_box(
                            buf,
                            sub_y as u32,
                            col_start,
                            col_width,
                            &native,
                            &sub_style,
                            &mut occupied,
                        );
                        (top_y, sub_y)
                    } else {
                        (top_y, bot_y)
                    }
                } else {
                    draw_ascii_active(
                        buf,
                        &active_line.text,
                        col_start,
                        col_width,
                        center_y,
                        min_y,
                        max_y,
                        effective_prog,
                        lyric_style,
                        palette,
                        &mut occupied,
                    )
                }
            };

            if show_text_lyrics {
                let gap = 2isize;
                if active_idx > 0 {
                    let mut y = banner_top - gap;
                    let mut idx = active_idx - 1;
                    let mut dist = 1isize;
                    while y >= min_y && idx >= 0 {
                        let style = context_line_color(dist);
                        let text =
                            truncate_chars(&lines[idx as usize].text, col_width.saturating_sub(4));
                        draw_text_in_box(
                            buf,
                            y as u32,
                            col_start,
                            col_width,
                            &text,
                            style,
                            &mut occupied,
                        );
                        y -= 1;
                        idx -= 1;
                        dist += 1;
                    }
                }

                let mut y = banner_bot + gap;
                let mut idx = (active_idx + 1).max(0);
                let mut dist = 1isize;
                while y <= max_y && (idx as usize) < lines.len() {
                    let style = context_line_color(dist);
                    let text =
                        truncate_chars(&lines[idx as usize].text, col_width.saturating_sub(4));
                    draw_text_in_box(
                        buf,
                        y as u32,
                        col_start,
                        col_width,
                        &text,
                        style,
                        &mut occupied,
                    );
                    y += 1;
                    idx += 1;
                    dist += 1;
                }
            }
        }
        LyricsPayload::Plain(lines) => {
            let (raw_active, _) = lyrics.active_index_and_progress(pos_secs, duration_secs);
            let active_idx =
                (raw_active.unwrap_or(0) as isize + manual_scroll).clamp(0, lines.len() as isize - 1);

            let (banner_top, banner_bot) = draw_ascii_active(
                buf,
                &lines[active_idx as usize],
                col_start,
                col_width,
                center_y,
                min_y,
                max_y,
                1.0,
                lyric_style,
                palette,
                &mut occupied,
            );

            if show_text_lyrics {
                let gap = 2isize;
                let mut y = banner_top - gap;
                let mut idx = active_idx - 1;
                let mut dist = 1isize;
                while y >= min_y && idx >= 0 {
                    let style = context_line_color(dist);
                    let text = truncate_chars(&lines[idx as usize], col_width.saturating_sub(4));
                    draw_text_in_box(
                        buf,
                        y as u32,
                        col_start,
                        col_width,
                        &text,
                        style,
                        &mut occupied,
                    );
                    y -= 1;
                    idx -= 1;
                    dist += 1;
                }

                let mut y = banner_bot + gap;
                let mut idx = active_idx + 1;
                let mut dist = 1isize;
                while y <= max_y && (idx as usize) < lines.len() {
                    let style = context_line_color(dist);
                    let text = truncate_chars(&lines[idx as usize], col_width.saturating_sub(4));
                    draw_text_in_box(
                        buf,
                        y as u32,
                        col_start,
                        col_width,
                        &text,
                        style,
                        &mut occupied,
                    );
                    y += 1;
                    idx += 1;
                    dist += 1;
                }
            }
        }
    }

    draw_starfield(
        buf, col_start, col_width, min_y, max_y, &occupied, palette, twinkle_t,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_ascii_active(
    buf: &mut String,
    text: &str,
    col_start: usize,
    col_width: usize,
    center_y: isize,
    min_y: isize,
    max_y: isize,
    progress: f64,
    lyric_style: LyricArtStyle,
    palette: ThemePalette,
    occupied: &mut Vec<(isize, usize, usize)>,
) -> (isize, isize) {
    if has_unsupported_glyphs(text) {
        return draw_unicode_karaoke_active(
            buf,
            text,
            col_start,
            col_width,
            center_y,
            min_y,
            max_y,
            progress,
            palette,
            occupied,
        );
    }

    let rendered = render_ascii_lyric(
        text,
        col_width.saturating_sub(2),
        progress,
        lyric_style,
        palette,
    );
    if rendered.rows.is_empty() {
        return (center_y, center_y);
    }

    let h = rendered.rows.len() as isize;
    let top_y = center_y - (h / 2);
    let bot_y = top_y + h - 1;

    for (i, (row_ansi, vis_w)) in rendered
        .rows
        .iter()
        .zip(rendered.visual_widths.iter())
        .enumerate()
    {
        let y = top_y + (i as isize);
        if y < min_y || y > max_y || *vis_w == 0 {
            continue;
        }
        let pad = col_width.saturating_sub(*vis_w) / 2;
        let x = col_start + pad;
        let _ = write!(buf, "\x1b[{y};{x}H{row_ansi}");
        occupied.push((y, x.saturating_sub(2), x + *vis_w + 2));
    }

    (top_y, bot_y)
}

#[allow(clippy::too_many_arguments)]
fn draw_unicode_karaoke_active(
    buf: &mut String,
    text: &str,
    col_start: usize,
    col_width: usize,
    center_y: isize,
    min_y: isize,
    max_y: isize,
    progress: f64,
    palette: ThemePalette,
    occupied: &mut Vec<(isize, usize, usize)>,
) -> (isize, isize) {
    let max_line_w = col_width.saturating_sub(6).max(12);
    let wrapped = wrap_unicode_lines(text.trim(), max_line_w, 3);
    if wrapped.is_empty() {
        return (center_y, center_y);
    }

    let total_chars: usize = wrapped.iter().map(|l| l.chars().count()).sum();
    if total_chars == 0 {
        return (center_y, center_y);
    }

    let h = wrapped.len() as isize;
    let top_y = center_y - (h / 2);
    let bot_y = top_y + h - 1;
    let sung_chars = ((total_chars as f64) * progress.clamp(0.0, 1.0)).round() as usize;
    let edge_char = sung_chars.saturating_sub(1);
    let [sr, sg, sb] = palette.sung;
    let [ar, ag, ab] = palette.accent;
    let sung_seq = format!("\x1b[1;38;2;{sr};{sg};{sb}m");
    let edge_seq = format!("\x1b[1;38;2;{ar};{ag};{ab}m");

    let mut global_idx = 0usize;
    for (row_idx, row_text) in wrapped.iter().enumerate() {
        let y = top_y + row_idx as isize;
        let vis_w = display_width(row_text);
        let pad = col_width.saturating_sub(vis_w) / 2;
        let x = col_start + pad;

        if y >= min_y && y <= max_y {
            let mut out = String::with_capacity(row_text.len() + 64);
            let mut current_zone: Option<u8> = None;

            for ch in row_text.chars() {
                let zone = if progress >= 1.0 || (progress > 0.0 && global_idx < edge_char) {
                    0
                } else if progress > 0.0 && global_idx <= sung_chars {
                    1
                } else {
                    2
                };
                if current_zone != Some(zone) {
                    match zone {
                        0 => out.push_str(&sung_seq),
                        1 => out.push_str(&edge_seq),
                        _ => out.push_str("\x1b[38;2;120;120;140m"),
                    }
                    current_zone = Some(zone);
                }
                out.push(ch);
                global_idx += 1;
            }
            out.push_str("\x1b[0m");
            let _ = write!(buf, "\x1b[{y};{x}H{out}");
            occupied.push((y, x.saturating_sub(2), x + vis_w + 2));
        } else {
            global_idx += row_text.chars().count();
        }
    }

    (top_y, bot_y)
}

#[allow(clippy::too_many_arguments)]
fn draw_starfield(
    buf: &mut String,
    col_start: usize,
    col_width: usize,
    min_y: isize,
    max_y: isize,
    occupied: &[(isize, usize, usize)],
    palette: ThemePalette,
    twinkle_t: f64,
) {
    if col_width < 12 || max_y < min_y + 2 {
        return;
    }

    const CELL_W: usize = 9;
    const CELL_H: isize = 3;
    let [ar, ag, ab] = palette.accent;
    let [sr, sg, sb] = palette.sung;

    let mut gy = min_y;
    while gy <= max_y {
        let mut gx = col_start;
        let col_end = col_start + col_width;
        while gx < col_end {
            let h = ((gx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                ^ ((gy as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
            if (h % 100) < 68 {
                let x = gx + ((h >> 4) % 7) as usize;
                let y = gy + ((h >> 10) % 3) as isize;
                if x >= col_start
                    && x < col_end
                    && y >= min_y
                    && y <= max_y
                    && !occupied
                        .iter()
                        .any(|&(oy, ox0, ox1)| oy == y && x >= ox0 && x < ox1)
                {
                    let speed = 0.85 + ((h >> 16) % 9) as f64 * 0.17;
                    let phase = ((h >> 24) % 628) as f64 * 0.01;
                    let wave = (twinkle_t * speed + phase).sin();

                    if wave >= -0.15 {
                        let (glyph, [r, g, b]) = if wave < 0.35 {
                            let ch = if (h >> 3) & 1 == 0 { '·' } else { '∘' };
                            (ch, blend_rgb([64, 68, 92], palette.accent, 0.18))
                        } else if wave < 0.78 {
                            let ch = if (h >> 5) & 1 == 0 { '⋆' } else { '✧' };
                            let tint = [(ar / 2) + (sr / 2), (ag / 2) + (sg / 2), (ab / 2) + (sb / 2)];
                            (ch, blend_rgb([92, 96, 122], tint, 0.48))
                        } else {
                            let ch = if (h >> 7) & 1 == 0 { '✦' } else { '✧' };
                            let tint = if (h >> 9) & 1 == 0 {
                                palette.accent
                            } else {
                                palette.sung
                            };
                            (ch, blend_rgb([120, 122, 148], tint, 0.68))
                        };
                        let _ = write!(buf, "\x1b[{y};{x}H\x1b[38;2;{r};{g};{b}m{glyph}\x1b[0m");
                    }
                }
            }
            gx += CELL_W;
        }
        gy += CELL_H;
    }
}

fn blend_rgb(base: [u8; 3], target: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        ((base[0] as f64) * (1.0 - t) + (target[0] as f64) * t).round() as u8,
        ((base[1] as f64) * (1.0 - t) + (target[1] as f64) * t).round() as u8,
        ((base[2] as f64) * (1.0 - t) + (target[2] as f64) * t).round() as u8,
    ]
}

fn char_display_width(ch: char) -> usize {
    let c = ch as u32;
    if (0x1100..=0x115F).contains(&c)
        || (0x2E80..=0xA4CF).contains(&c)
        || (0xAC00..=0xD7A3).contains(&c)
        || (0xF900..=0xFAFF).contains(&c)
        || (0xFE10..=0xFE6F).contains(&c)
        || (0xFF00..=0xFF60).contains(&c)
        || (0xFFE0..=0xFFE6).contains(&c)
        || (0x20000..=0x3FFFF).contains(&c)
    {
        2
    } else {
        1
    }
}

fn display_width(text: &str) -> usize {
    text.chars().map(char_display_width).sum()
}

fn wrap_unicode_lines(text: &str, max_w: usize, max_lines: usize) -> Vec<String> {
    if text.is_empty() || max_w == 0 || max_lines == 0 {
        return Vec::new();
    }

    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    let mut truncated = false;

    for token in text.split_whitespace() {
        let tok_w = display_width(token);
        let need_space = if cur.is_empty() { 0 } else { 1 };

        if cur_w + need_space + tok_w <= max_w {
            if need_space == 1 {
                cur.push(' ');
                cur_w += 1;
            }
            cur.push_str(token);
            cur_w += tok_w;
            continue;
        }

        if !cur.is_empty() {
            lines.push(cur);
            cur = String::new();
            cur_w = 0;
            if lines.len() >= max_lines {
                truncated = true;
                break;
            }
        }

        if tok_w <= max_w {
            cur.push_str(token);
            cur_w = tok_w;
        } else {
            for ch in token.chars() {
                let cw = char_display_width(ch);
                if cur_w + cw > max_w && !cur.is_empty() {
                    lines.push(cur);
                    cur = String::new();
                    cur_w = 0;
                    if lines.len() >= max_lines {
                        truncated = true;
                        break;
                    }
                }
                cur.push(ch);
                cur_w += cw;
            }
            if truncated {
                break;
            }
        }
    }

    if !truncated && !cur.is_empty() {
        if lines.len() < max_lines {
            lines.push(cur);
        } else {
            truncated = true;
        }
    }

    if truncated && let Some(last) = lines.last_mut() {
        while display_width(last) + 1 > max_w && !last.is_empty() {
            last.pop();
        }
        last.push('…');
    }

    lines
}

fn context_line_color(dist: isize) -> &'static str {
    match dist {
        1 => "\x1b[38;2;188;188;202m",
        2 => "\x1b[38;2;132;132;148m",
        3 => "\x1b[38;2;96;96;112m",
        _ => "\x1b[38;2;70;70;84m",
    }
}

fn draw_text_in_box(
    buf: &mut String,
    row_y: u32,
    col_start: usize,
    col_width: usize,
    text: &str,
    ansi_style: &str,
    occupied: &mut Vec<(isize, usize, usize)>,
) {
    let vis_w = display_width(text);
    let pad = col_width.saturating_sub(vis_w) / 2;
    let x = col_start + pad;
    let _ = write!(buf, "\x1b[{row_y};{x}H{ansi_style}{text}\x1b[0m");
    occupied.push((row_y as isize, x.saturating_sub(2), x + vis_w + 2));
}

fn draw_centered_text(buf: &mut String, row_y: u32, term_w: u32, text: &str, ansi_style: &str) {
    let mut ignored = Vec::new();
    draw_text_in_box(buf, row_y, 1, term_w as usize, text, ansi_style, &mut ignored);
}

fn format_progress_bar(
    pos_secs: f64,
    dur_secs: f64,
    total_width: usize,
    status: PlaybackStatus,
    palette: ThemePalette,
) -> String {
    let cur_str = format_mm_ss(pos_secs);
    let tot_str = format_mm_ss(dur_secs);
    let icon = match status {
        PlaybackStatus::Playing => "▶",
        PlaybackStatus::Paused => "⏸",
        PlaybackStatus::Stopped => "⏹",
    };

    let prefix = format!("{icon} {cur_str} ");
    let suffix = format!(" {tot_str}");
    let bar_len = total_width.saturating_sub(prefix.chars().count() + suffix.chars().count());

    if bar_len < 4 {
        return format!("\x1b[38;2;168;168;180m{prefix}{tot_str}\x1b[0m");
    }

    let ratio = if dur_secs > 0.5 {
        (pos_secs / dur_secs).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((bar_len as f64) * ratio).round() as usize;
    let empty = bar_len.saturating_sub(filled);
    let [ar, ag, ab] = palette.accent;

    let mut out = String::new();
    let _ = write!(
        out,
        "\x1b[38;2;180;180;192m{prefix}\x1b[38;2;{ar};{ag};{ab}m{}\x1b[38;2;75;75;90m{}\x1b[38;2;180;180;192m{suffix}\x1b[0m",
        "━".repeat(filled),
        "─".repeat(empty)
    );
    out
}

#[allow(clippy::too_many_arguments)]
fn draw_hud_bar(
    buf: &mut String,
    term_w: u32,
    term_h: u32,
    cover_style: CoverStyle,
    lyric_style: LyricArtStyle,
    show_text_lyrics: bool,
    romanize: bool,
    dynamic_color: bool,
    offset_ms: i64,
) {
    let offset_tag = if offset_ms == 0 {
        String::new()
    } else {
        format!(" | Offset:{offset_ms:+}ms")
    };
    let text_state = if show_text_lyrics { "On" } else { "Off" };
    let roma_state = if romanize { "On" } else { "Off" };
    let theme_state = if dynamic_color { "Dyn" } else { "Def" };
    let text = format!(
        "Space:Play | n/p:Lagu | c/v:Cover({}) | f:ASCII({}) | r:Roma({}) | d:Tema({}) | t:Teks({}) | R:Refetch | [/]:Offset{} | h:Hide | q:Quit",
        cover_style.label(),
        lyric_style.label(),
        roma_state,
        theme_state,
        text_state,
        offset_tag
    );
    let trunc = truncate_chars(&text, (term_w as usize).saturating_sub(4));
    let char_len = trunc.chars().count() + 2;
    let col = ((term_w as usize).saturating_sub(char_len) / 2) + 1;
    let _ = write!(
        buf,
        "\x1b[{term_h};{col}H\x1b[48;2;16;16;22m\x1b[38;2;185;185;198m {trunc} \x1b[0m"
    );
}

fn format_mm_ss(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = input.chars().count();
    if count <= max_chars {
        return input.to_string();
    }
    let mut s: String = input.chars().take(max_chars.saturating_sub(1)).collect();
    s.push('…');
    s
}

fn terminal_size() -> (u32, u32) {
    crossterm::terminal::size()
        .map(|(w, h)| (w as u32, h as u32))
        .unwrap_or((100, 30))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_long_unicode_lines_up_to_three_rows() {
        let cjk = "あいうえおかきくけこさしすせそたちつてとなにぬね";
        let wrapped = wrap_unicode_lines(cjk, 14, 3);
        assert_eq!(wrapped.len(), 3);
        for line in &wrapped {
            assert!(display_width(line) <= 14);
        }
        assert!(wrapped[2].ends_with('…'));

        let short = "사랑해 Hurt";
        assert_eq!(wrap_unicode_lines(short, 30, 3), vec!["사랑해 Hurt"]);
    }
}
