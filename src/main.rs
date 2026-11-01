mod ascii_font;
mod cli;
mod cover;
mod lyrics;
mod player;
mod ui;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, CoverStyle};
use crate::cover::{AsciiCover, load_and_render_cover};
use crate::lyrics::{
    LyricsPayload, fetch_lyrics_by_query, fetch_lyrics_for_track, print_lyrics_stdout,
};
use crate::player::{
    PlaybackStatus, TrackState, player_next, player_play_pause, player_previous,
    player_seek_relative, poll_active_track,
};
use crate::ui::{TerminalUi, UserAction, poll_user_action};

enum WorkerEvent {
    LyricsReady {
        cache_key: String,
        payload: LyricsPayload,
    },
    CoverReady {
        cache_key: String,
        style: CoverStyle,
        cover: Option<AsciiCover>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(track_title) = &cli.track {
        let payload = fetch_lyrics_by_query(cli.artist.as_deref(), track_title);
        if cli.print {
            return print_lyrics_stdout(
                track_title,
                cli.artist.as_deref().unwrap_or("Unknown Artist"),
                &payload,
            );
        }
        return run_static_query_tui(&cli, track_title, cli.artist.as_deref(), payload);
    }

    if cli.print {
        let Some(track) = poll_active_track(cli.player.as_deref()) else {
            println!("Tidak ada pemutar musik aktif yang terdeteksi.");
            return Ok(());
        };
        let payload = fetch_lyrics_for_track(&track);
        return print_lyrics_stdout(&track.title, &track.artist, &payload);
    }

    run_live_tui(cli)
}

fn run_static_query_tui(
    cli: &Cli,
    title: &str,
    artist: Option<&str>,
    payload: LyricsPayload,
) -> Result<()> {
    let mut ui = TerminalUi::enter()?;
    let mut hide_hud = cli.hide_controls;
    let mut lyric_style = cli.lyric_style;
    let mut show_text_lyrics = cli.show_text && !cli.hide_text;
    let mut manual_scroll: isize = 0;

    let synth_track = TrackState {
        player_name: "search".to_string(),
        status: PlaybackStatus::Paused,
        track_id: "search".to_string(),
        title: title.to_string(),
        artist: artist.unwrap_or("Pencarian Manual").to_string(),
        album: String::new(),
        duration_secs: 0.0,
        position_secs: 0.0,
        art_url: None,
    };

    loop {
        ui.render_track(
            &synth_track,
            0.0,
            &payload,
            None,
            false,
            cli.charset,
            lyric_style,
            show_text_lyrics,
            hide_hud,
            0,
            manual_scroll,
            false,
        )?;

        match poll_user_action(Duration::from_millis(80))? {
            UserAction::Quit => break,
            UserAction::ToggleHud => hide_hud = !hide_hud,
            UserAction::CycleLyricStyle => lyric_style = lyric_style.next(),
            UserAction::ToggleTextLyrics => show_text_lyrics = !show_text_lyrics,
            UserAction::ScrollUp => manual_scroll = manual_scroll.saturating_sub(1),
            UserAction::ScrollDown => manual_scroll = manual_scroll.saturating_add(1),
            _ => {}
        }
    }

    Ok(())
}

fn run_live_tui(cli: Cli) -> Result<()> {
    let mut ui = TerminalUi::enter()?;
    let (tx, rx) = mpsc::channel::<WorkerEvent>();

    let mut show_cover = !cli.no_cover;
    let mut cover_style = cli.charset;
    let mut lyric_style = cli.lyric_style;
    let mut show_text_lyrics = cli.show_text && !cli.hide_text;
    let mut hide_hud = cli.hide_controls;
    let mut offset_ms = cli.offset;
    let mut manual_scroll: isize = 0;

    let mut current_track: Option<TrackState> = None;
    let mut current_key = String::new();
    let mut current_lyrics = LyricsPayload::NotFound;
    let mut current_cover: Option<AsciiCover> = None;
    let mut loading_lyrics = false;

    let mut last_poll = Instant::now() - Duration::from_secs(5);
    let mut last_pos_sync = Instant::now();
    let mut base_pos_secs = 0.0;

    loop {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                WorkerEvent::LyricsReady { cache_key, payload } => {
                    if cache_key == current_key {
                        current_lyrics = payload;
                        loading_lyrics = false;
                    }
                }
                WorkerEvent::CoverReady {
                    cache_key,
                    style,
                    cover,
                } => {
                    if cache_key == current_key && style == cover_style {
                        current_cover = cover;
                    }
                }
            }
        }

        if last_poll.elapsed() >= Duration::from_millis(450) {
            last_poll = Instant::now();
            if let Some(polled) = poll_active_track(cli.player.as_deref()) {
                let key = polled.cache_key();
                if key != current_key {
                    current_key = key.clone();
                    current_lyrics = LyricsPayload::NotFound;
                    current_cover = None;
                    loading_lyrics = true;
                    manual_scroll = 0;

                    let tx_lyrics = tx.clone();
                    let track_clone = polled.clone();
                    let key_clone = key.clone();
                    thread::spawn(move || {
                        let payload = fetch_lyrics_for_track(&track_clone);
                        let _ = tx_lyrics.send(WorkerEvent::LyricsReady {
                            cache_key: key_clone,
                            payload,
                        });
                    });

                    if let Some(art_url) = polled.art_url.clone() {
                        let tx_cov = tx.clone();
                        let key_cov = key;
                        let style = cover_style;
                        let cols = cli.cover_width;
                        thread::spawn(move || {
                            let cover = load_and_render_cover(&art_url, cols, style);
                            let _ = tx_cov.send(WorkerEvent::CoverReady {
                                cache_key: key_cov,
                                style,
                                cover,
                            });
                        });
                    }
                }
                base_pos_secs = polled.position_secs;
                last_pos_sync = Instant::now();
                current_track = Some(polled);
            } else {
                current_track = None;
                current_key.clear();
            }
        }

        if let Some(track) = &current_track {
            let live_pos = if track.status == PlaybackStatus::Playing {
                (base_pos_secs + last_pos_sync.elapsed().as_secs_f64()).min(track.duration_secs)
            } else {
                base_pos_secs
            };

            ui.render_track(
                track,
                live_pos,
                &current_lyrics,
                current_cover.as_ref(),
                show_cover,
                cover_style,
                lyric_style,
                show_text_lyrics,
                hide_hud,
                offset_ms,
                manual_scroll,
                loading_lyrics,
            )?;
        } else {
            ui.render_waiting(hide_hud, cover_style, lyric_style, show_text_lyrics)?;
        }

        match poll_user_action(Duration::from_millis(50))? {
            UserAction::Quit => break,
            UserAction::ToggleCover => show_cover = !show_cover,
            UserAction::CycleCoverStyle => {
                cover_style = cover_style.next();
                if let Some(track) = &current_track
                    && let Some(art_url) = track.art_url.clone()
                {
                    let tx_cov = tx.clone();
                    let key_cov = current_key.clone();
                    let style = cover_style;
                    let cols = cli.cover_width;
                    thread::spawn(move || {
                        let cover = load_and_render_cover(&art_url, cols, style);
                        let _ = tx_cov.send(WorkerEvent::CoverReady {
                            cache_key: key_cov,
                            style,
                            cover,
                        });
                    });
                }
            }
            UserAction::CycleLyricStyle => lyric_style = lyric_style.next(),
            UserAction::ToggleTextLyrics => show_text_lyrics = !show_text_lyrics,
            UserAction::ToggleHud => hide_hud = !hide_hud,
            UserAction::OffsetEarlier => offset_ms -= 250,
            UserAction::OffsetLater => offset_ms += 250,
            UserAction::ScrollUp => manual_scroll = manual_scroll.saturating_sub(1),
            UserAction::ScrollDown => manual_scroll = manual_scroll.saturating_add(1),
            UserAction::PlayPause => {
                if let Some(track) = &current_track {
                    player_play_pause(&track.player_name);
                    last_poll = Instant::now() - Duration::from_secs(5);
                }
            }
            UserAction::NextTrack => {
                if let Some(track) = &current_track {
                    player_next(&track.player_name);
                    last_poll = Instant::now() - Duration::from_secs(5);
                }
            }
            UserAction::PrevTrack => {
                if let Some(track) = &current_track {
                    player_previous(&track.player_name);
                    last_poll = Instant::now() - Duration::from_secs(5);
                }
            }
            UserAction::SeekForward => {
                if let Some(track) = &current_track {
                    player_seek_relative(&track.player_name, 5);
                    last_poll = Instant::now() - Duration::from_secs(5);
                }
            }
            UserAction::SeekBackward => {
                if let Some(track) = &current_track {
                    player_seek_relative(&track.player_name, -5);
                    last_poll = Instant::now() - Duration::from_secs(5);
                }
            }
            UserAction::None => {}
        }
    }

    Ok(())
}
