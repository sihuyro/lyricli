use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq)]
pub struct TrackState {
    pub player_name: String,
    pub status: PlaybackStatus,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: f64,
    pub position_secs: f64,
    pub art_url: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

impl TrackState {
    pub fn cache_key(&self) -> String {
        let raw = format!(
            "{}_{}_{}_{}",
            self.artist.trim().to_ascii_lowercase(),
            self.title.trim().to_ascii_lowercase(),
            self.album.trim().to_ascii_lowercase(),
            self.duration_secs.round() as u64
        );
        let hash = fnv1a_64(raw.as_bytes());
        format!("{}_{hash:016x}", sanitize_filename(&raw))
    }
}

pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn sanitize_filename(input: &str) -> String {
    let mut out = String::with_capacity(input.len().min(120));
    for ch in input.chars() {
        if out.len() >= 100 {
            break;
        }
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown_track".to_string()
    } else {
        out
    }
}

pub fn poll_active_track(preferred_player: Option<&str>) -> Option<TrackState> {
    let players = list_mpris_players();
    if players.is_empty() {
        return None;
    }

    let ordered = order_players(&players, preferred_player);

    let mut fallback_paused: Option<TrackState> = None;
    for player in ordered {
        if let Some(state) = query_player(&player) {
            if state.status == PlaybackStatus::Playing {
                return Some(state);
            }
            if fallback_paused.is_none() && !state.title.is_empty() {
                fallback_paused = Some(state);
            }
        }
    }

    fallback_paused
}

fn list_mpris_players() -> Vec<String> {
    let Ok(out) = Command::new("playerctl")
        .arg("-l")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn order_players(players: &[String], preferred: Option<&str>) -> Vec<String> {
    if let Some(pref) = preferred {
        let pref_lower = pref.to_ascii_lowercase();
        let mut matched: Vec<String> = players
            .iter()
            .filter(|p| p.to_ascii_lowercase().contains(&pref_lower))
            .cloned()
            .collect();
        if matched.is_empty() {
            matched.push(pref.to_string());
        }
        return matched;
    }

    let mut spotify_first = Vec::with_capacity(players.len());
    for p in players {
        if p.to_ascii_lowercase().contains("spotify") {
            spotify_first.push(p.clone());
        }
    }
    for p in players {
        if !p.to_ascii_lowercase().contains("spotify") {
            spotify_first.push(p.clone());
        }
    }
    spotify_first
}

fn query_player(player: &str) -> Option<TrackState> {
    let fmt = "{{status}}\t{{position}}\t{{mpris:length}}\t{{mpris:trackid}}\t{{xesam:title}}\t{{xesam:artist}}\t{{xesam:album}}\t{{mpris:artUrl}}";
    let out = Command::new("playerctl")
        .args(["-p", player, "metadata", "--format", fmt])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let raw = String::from_utf8_lossy(&out.stdout);
    let line = raw.lines().next()?.trim_end_matches(['\r', '\n']);
    let fields: Vec<&str> = line.splitn(8, '\t').collect();
    if fields.len() < 8 {
        return None;
    }

    let status = match fields[0].trim() {
        "Playing" => PlaybackStatus::Playing,
        "Paused" => PlaybackStatus::Paused,
        _ => PlaybackStatus::Stopped,
    };

    let pos_us: f64 = fields[1].trim().parse().unwrap_or(0.0);
    let len_us: f64 = fields[2].trim().parse().unwrap_or(0.0);
    let position_secs = (pos_us / 1_000_000.0).max(0.0);
    let duration_secs = (len_us / 1_000_000.0).max(0.0);

    let track_id = fields[3].trim().to_string();
    let title = fields[4].trim().to_string();
    let artist = fields[5].trim().to_string();
    let album = fields[6].trim().to_string();
    let art_raw = fields[7].trim();
    let art_url = if art_raw.is_empty() {
        None
    } else {
        Some(art_raw.to_string())
    };

    if title.is_empty() {
        return None;
    }

    Some(TrackState {
        player_name: player.to_string(),
        status,
        track_id,
        title,
        artist,
        album,
        duration_secs,
        position_secs,
        art_url,
    })
}

pub fn player_play_pause(player: &str) {
    let _ = Command::new("playerctl")
        .args(["-p", player, "play-pause"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn player_next(player: &str) {
    let _ = Command::new("playerctl")
        .args(["-p", player, "next"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn player_previous(player: &str) {
    let _ = Command::new("playerctl")
        .args(["-p", player, "previous"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn player_seek_relative(player: &str, delta_secs: i64) {
    let arg = if delta_secs >= 0 {
        format!("{delta_secs}+")
    } else {
        format!("{}-", delta_secs.unsigned_abs())
    };
    let _ = Command::new("playerctl")
        .args(["-p", player, "position", &arg])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
