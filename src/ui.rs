use std::fmt::Write as FmtWrite;
use std::io::{self, Write};
use std::time::Duration;

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
use crate::cover::AsciiCover;
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
}

impl TerminalUi {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All))?;
        Ok(Self {
            buf: String::with_capacity(65536),
        })
    }

    pub fn render_waiting(
        &mut self,
        hide_hud: bool,
        cover_style: CoverStyle,
        lyric_style: LyricArtStyle,
        show_text_lyrics: bool,
    ) -> Result<()> {
        let (term_w, term_h) = terminal_size();
        self.buf.clear();
        self.buf.push_str("\x1b[2J");

        let center_y = (term_h / 2).max(1) as isize;
        draw_ascii_active(
            &mut self.buf,
            "WAITING FOR SPOTIFY",
            1,
            term_w as usize,
            center_y,
            1,
            term_h as isize,
            1.0,
            lyric_style,
        );

        if !hide_hud {
            draw_hud_bar(
                &mut self.buf,
                term_w,
                term_h,
                cover_style,
                lyric_style,
                show_text_lyrics,
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
        hide_hud: bool,
        offset_ms: i64,
        manual_scroll: isize,
        loading_lyrics: bool,
    ) -> Result<()> {
        let (term_w, term_h) = terminal_size();
        self.buf.clear();
        self.buf.push_str("\x1b[2J");

        let adjusted_pos = (pos_secs - (offset_ms as f64 / 1000.0)).max(0.0);

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

        let meta_y = left_top_y + cover.height + 1;
        if meta_y <= usable_h {
            let title_trunc = truncate_chars(&track.title, cov_w);
            let title_pad = cov_w.saturating_sub(title_trunc.chars().count()) / 2;
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
            let sub_pad = cov_w.saturating_sub(sub_trunc.chars().count()) / 2;
            let _ = write!(
                self.buf,
                "\x1b[{};{}H\x1b[38;2;242;109;111m{}\x1b[0m",
                meta_y + 1,
                left_x + sub_pad,
                sub_trunc
            );
        }
        if meta_y + 2 <= usable_h {
            let prog = format_progress_bar(pos_secs, track.duration_secs, cov_w, track.status);
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

        let header_line = format!("{}  ·  {}", track.title, track.artist);
        let header_trunc = truncate_chars(&header_line, max_w);
        draw_centered_text(
            &mut self.buf,
            2,
            term_w,
            &header_trunc,
            "\x1b[1;38;2;242;109;111m",
        );

        let bar_w = max_w.min(48);
        let prog = format_progress_bar(pos_secs, track.duration_secs, bar_w, track.status);
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
    manual_scroll: isize,
    loading_lyrics: bool,
) {
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
                )
            } else {
                let active_text = &lines[active_idx as usize].text;
                let effective_prog = if manual_scroll == 0 { progress } else { 1.0 };
                draw_ascii_active(
                    buf,
                    active_text,
                    col_start,
                    col_width,
                    center_y,
                    min_y,
                    max_y,
                    effective_prog,
                    lyric_style,
                )
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
                            truncate_chars(&lines[idx as usize].text, col_width.saturating_sub(2));
                        draw_text_in_box(buf, y as u32, col_start, col_width, &text, style);
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
                        truncate_chars(&lines[idx as usize].text, col_width.saturating_sub(2));
                    draw_text_in_box(buf, y as u32, col_start, col_width, &text, style);
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
            );

            if show_text_lyrics {
                let gap = 2isize;
                let mut y = banner_top - gap;
                let mut idx = active_idx - 1;
                let mut dist = 1isize;
                while y >= min_y && idx >= 0 {
                    let style = context_line_color(dist);
                    let text = truncate_chars(&lines[idx as usize], col_width.saturating_sub(2));
                    draw_text_in_box(buf, y as u32, col_start, col_width, &text, style);
                    y -= 1;
                    idx -= 1;
                    dist += 1;
                }

                let mut y = banner_bot + gap;
                let mut idx = active_idx + 1;
                let mut dist = 1isize;
                while y <= max_y && (idx as usize) < lines.len() {
                    let style = context_line_color(dist);
                    let text = truncate_chars(&lines[idx as usize], col_width.saturating_sub(2));
                    draw_text_in_box(buf, y as u32, col_start, col_width, &text, style);
                    y += 1;
                    idx += 1;
                    dist += 1;
                }
            }
        }
    }
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
        );
    }

    let rendered = render_ascii_lyric(text, col_width.saturating_sub(2), progress, lyric_style);
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
) -> (isize, isize) {
    let clean = truncate_chars(text.trim(), col_width.saturating_sub(4));
    let total_chars = clean.chars().count();
    if total_chars == 0 {
        return (center_y, center_y);
    }
    let vis_w = clean
        .chars()
        .map(|c| if c as u32 >= 0x1100 { 2 } else { 1 })
        .sum::<usize>();
    let pad = col_width.saturating_sub(vis_w) / 2;
    let x = col_start + pad;

    if center_y >= min_y && center_y <= max_y {
        let sung_chars = ((total_chars as f64) * progress.clamp(0.0, 1.0)).round() as usize;
        let edge_char = sung_chars.saturating_sub(1);
        let mut line = String::with_capacity(clean.len() + 64);
        let mut current_zone: Option<u8> = None;

        for (i, ch) in clean.chars().enumerate() {
            let zone = if progress >= 1.0 || (progress > 0.0 && i < edge_char) {
                0
            } else if progress > 0.0 && i <= sung_chars {
                1
            } else {
                2
            };
            if current_zone != Some(zone) {
                match zone {
                    0 => line.push_str("\x1b[1;38;2;250;250;255m"),
                    1 => line.push_str("\x1b[1;38;2;242;109;111m"),
                    _ => line.push_str("\x1b[38;2;120;120;140m"),
                }
                current_zone = Some(zone);
            }
            line.push(ch);
        }
        line.push_str("\x1b[0m");
        let _ = write!(buf, "\x1b[{center_y};{x}H{line}");
    }

    (center_y, center_y)
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
) {
    let char_len = text.chars().count();
    let pad = col_width.saturating_sub(char_len) / 2;
    let x = col_start + pad;
    let _ = write!(buf, "\x1b[{row_y};{x}H{ansi_style}{text}\x1b[0m");
}

fn draw_centered_text(buf: &mut String, row_y: u32, term_w: u32, text: &str, ansi_style: &str) {
    draw_text_in_box(buf, row_y, 1, term_w as usize, text, ansi_style);
}

fn format_progress_bar(
    pos_secs: f64,
    dur_secs: f64,
    total_width: usize,
    status: PlaybackStatus,
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

    let mut out = String::new();
    let _ = write!(
        out,
        "\x1b[38;2;180;180;192m{prefix}\x1b[38;2;198;40;40m{}\x1b[38;2;75;75;90m{}\x1b[38;2;180;180;192m{suffix}\x1b[0m",
        "━".repeat(filled),
        "─".repeat(empty)
    );
    out
}

fn draw_hud_bar(
    buf: &mut String,
    term_w: u32,
    term_h: u32,
    cover_style: CoverStyle,
    lyric_style: LyricArtStyle,
    show_text_lyrics: bool,
    offset_ms: i64,
) {
    let offset_tag = if offset_ms == 0 {
        String::new()
    } else {
        format!(" | Offset:{offset_ms:+}ms")
    };
    let text_state = if show_text_lyrics { "On" } else { "Off" };
    let text = format!(
        "Space:Play/Pause | n/p:Lagu | c:Cover | v:Cover({}) | f:ASCII({}) | t:Teks({}) | [/]:Offset{} | h:Hide | q:Quit",
        cover_style.label(),
        lyric_style.label(),
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
