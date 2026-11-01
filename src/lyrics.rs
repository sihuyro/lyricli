use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::player::{TrackState, sanitize_filename};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LyricLine {
    pub time_secs: Option<f64>,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LyricsPayload {
    Synced(Vec<LyricLine>),
    Plain(Vec<String>),
    Instrumental,
    NotFound,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrclibResponse {
    instrumental: Option<bool>,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
    duration: Option<f64>,
}

impl LyricsPayload {
    pub fn active_index(&self, pos_secs: f64, total_duration: f64) -> Option<usize> {
        match self {
            Self::Synced(lines) => {
                if lines.is_empty() {
                    return None;
                }
                let mut active = None;
                for (i, line) in lines.iter().enumerate() {
                    if let Some(t) = line.time_secs {
                        if pos_secs + 0.08 >= t {
                            active = Some(i);
                        } else {
                            break;
                        }
                    }
                }
                active
            }
            Self::Plain(lines) => {
                if lines.is_empty() || total_duration <= 1.0 {
                    return Some(0);
                }
                let ratio = (pos_secs / total_duration).clamp(0.0, 0.999);
                Some(((lines.len() as f64) * ratio).floor() as usize)
            }
            _ => None,
        }
    }

    pub fn active_index_and_progress(
        &self,
        pos_secs: f64,
        total_duration: f64,
    ) -> (Option<usize>, f64) {
        let Some(idx) = self.active_index(pos_secs, total_duration) else {
            return (None, 0.0);
        };
        match self {
            Self::Synced(lines) => {
                let start_t = lines.get(idx).and_then(|l| l.time_secs).unwrap_or(0.0);
                let next_t = lines
                    .get(idx + 1)
                    .and_then(|l| l.time_secs)
                    .unwrap_or_else(|| (start_t + 4.0).min(total_duration.max(start_t + 4.0)));
                let span = (next_t - start_t).clamp(0.4, 5.5);
                let prog = ((pos_secs - start_t) / span).clamp(0.0, 1.0);
                (Some(idx), prog)
            }
            Self::Plain(_) => (Some(idx), 1.0),
            _ => (None, 0.0),
        }
    }
}

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("lyricli");
    let _ = fs::create_dir_all(dir.join("lyrics"));
    let _ = fs::create_dir_all(dir.join("covers"));
    dir
}

pub fn fetch_lyrics_for_track(track: &TrackState) -> LyricsPayload {
    let key = track.cache_key();
    let cache_path = cache_dir().join("lyrics").join(format!("{key}.json"));

    if let Ok(cached_bytes) = fs::read(&cache_path)
        && let Ok(payload) = serde_json::from_slice::<LyricsPayload>(&cached_bytes)
        && !matches!(payload, LyricsPayload::NotFound)
    {
        return payload;
    }

    let payload = query_lrclib(&track.artist, &track.title, &track.album, track.duration_secs);
    if !matches!(payload, LyricsPayload::NotFound)
        && let Ok(json) = serde_json::to_vec(&payload)
    {
        let _ = fs::write(&cache_path, json);
    }
    payload
}

pub fn fetch_lyrics_by_query(artist: Option<&str>, title: &str) -> LyricsPayload {
    let artist_str = artist.unwrap_or("");
    let key = sanitize_filename(&format!("search_{artist_str}_{title}"));
    let cache_path = cache_dir().join("lyrics").join(format!("{key}.json"));

    if let Ok(cached_bytes) = fs::read(&cache_path)
        && let Ok(payload) = serde_json::from_slice::<LyricsPayload>(&cached_bytes)
        && !matches!(payload, LyricsPayload::NotFound)
    {
        return payload;
    }

    let payload = query_lrclib(artist_str, title, "", 0.0);
    if !matches!(payload, LyricsPayload::NotFound)
        && let Ok(json) = serde_json::to_vec(&payload)
    {
        let _ = fs::write(&cache_path, json);
    }
    payload
}

fn query_lrclib(artist: &str, title: &str, album: &str, duration_secs: f64) -> LyricsPayload {
    if !artist.is_empty()
        && !title.is_empty()
        && let Some(resp) = lrclib_get_exact(artist, title, album, duration_secs)
    {
        let parsed = convert_lrclib_response(resp);
        if !matches!(parsed, LyricsPayload::NotFound) {
            return parsed;
        }
    }

    let clean_title = strip_track_noise(title);
    let primary_artist = artist.split(',').next().unwrap_or(artist).trim();

    let queries = [
        format!("{primary_artist} {title}").trim().to_string(),
        format!("{primary_artist} {clean_title}").trim().to_string(),
        clean_title,
    ];

    for q in queries {
        if q.is_empty() {
            continue;
        }
        if let Some(best) = lrclib_search(&q, duration_secs) {
            let parsed = convert_lrclib_response(best);
            if !matches!(parsed, LyricsPayload::NotFound) {
                return parsed;
            }
        }
    }

    LyricsPayload::NotFound
}

fn lrclib_get_exact(
    artist: &str,
    title: &str,
    album: &str,
    duration_secs: f64,
) -> Option<LrclibResponse> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--max-time",
        "6",
        "-A",
        "lyricli/0.1.0 (https://github.com/sihuyro/lyricli)",
        "-G",
        "https://lrclib.net/api/get",
    ])
    .arg("--data-urlencode")
    .arg(format!("artist_name={artist}"))
    .arg("--data-urlencode")
    .arg(format!("track_name={title}"));

    if !album.is_empty() {
        cmd.arg("--data-urlencode")
            .arg(format!("album_name={album}"));
    }
    if duration_secs > 1.0 {
        cmd.arg("--data-urlencode")
            .arg(format!("duration={}", duration_secs.round() as u64));
    }

    let out = cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice::<LrclibResponse>(&out.stdout).ok()
}

fn lrclib_search(query: &str, target_duration: f64) -> Option<LrclibResponse> {
    let out = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "6",
            "-A",
            "lyricli/0.1.0 (https://github.com/abdulinux/lyricli)",
            "-G",
            "https://lrclib.net/api/search",
            "--data-urlencode",
        ])
        .arg(format!("q={query}"))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let items: Vec<LrclibResponse> = serde_json::from_slice(&out.stdout).ok()?;
    if items.is_empty() {
        return None;
    }

    items.into_iter().max_by_key(|item| {
        let has_synced = item
            .synced_lyrics
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
        let has_plain = item
            .plain_lyrics
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
        let dur_score = if target_duration > 1.0 {
            let d = item.duration.unwrap_or(0.0);
            let diff = (d - target_duration).abs();
            if diff <= 3.0 {
                50
            } else if diff <= 10.0 {
                25
            } else {
                0
            }
        } else {
            0
        };
        (if has_synced { 100 } else if has_plain { 40 } else { 0 }) + dur_score
    })
}

fn convert_lrclib_response(resp: LrclibResponse) -> LyricsPayload {
    if resp.instrumental == Some(true) {
        return LyricsPayload::Instrumental;
    }
    if let Some(synced) = resp.synced_lyrics {
        let lines = parse_lrc(&synced);
        if !lines.is_empty() {
            return LyricsPayload::Synced(lines);
        }
    }
    if let Some(plain) = resp.plain_lyrics {
        let lines: Vec<String> = plain
            .lines()
            .map(|l| l.trim().to_string())
            .collect();
        if lines.iter().any(|l| !l.is_empty()) {
            return LyricsPayload::Plain(lines);
        }
    }
    LyricsPayload::NotFound
}

pub fn parse_lrc(lrc_text: &str) -> Vec<LyricLine> {
    let mut result = Vec::new();

    for raw_line in lrc_text.lines() {
        let mut rest = raw_line.trim();
        let mut timestamps = Vec::new();

        while let Some( after_open) = rest.strip_prefix('[') {
            let Some(close_idx) = after_open.find(']') else {
                break;
            };
            let tag = &after_open[..close_idx];
            if let Some(secs) = parse_timestamp_tag(tag) {
                timestamps.push(secs);
                rest = after_open[close_idx + 1..].trim_start();
            } else {
                break;
            }
        }

        if timestamps.is_empty() {
            continue;
        }

        let text = if rest.is_empty() {
            "♪".to_string()
        } else {
            rest.to_string()
        };

        for t in timestamps {
            result.push(LyricLine {
                time_secs: Some(t),
                text: text.clone(),
            });
        }
    }

    result.sort_by(|a, b| {
        a.time_secs
            .unwrap_or(0.0)
            .partial_cmp(&b.time_secs.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    result
}

fn parse_timestamp_tag(tag: &str) -> Option<f64> {
    let (mins_str, secs_str) = tag.split_once(':')?;
    let mins: f64 = mins_str.trim().parse().ok()?;
    let secs: f64 = secs_str.trim().replace(',', ".").parse().ok()?;
    Some((mins * 60.0 + secs).max(0.0))
}

fn strip_track_noise(title: &str) -> String {
    let mut s = title.to_string();
    if let Some((left, _)) = s.split_once(" - ") {
        s = left.to_string();
    }
    for (open, close) in [('(', ')'), ('[', ']')] {
        while let Some(start) = s.find(open) {
            if let Some(end_rel) = s[start..].find(close) {
                let inner = s[start + 1..start + end_rel].to_ascii_lowercase();
                if inner.contains("remaster")
                    || inner.contains("feat")
                    || inner.contains("ft.")
                    || inner.contains("version")
                    || inner.contains("live")
                    || inner.contains("edit")
                {
                    s.replace_range(start..=start + end_rel, "");
                    continue;
                }
            }
            break;
        }
    }
    s.trim().to_string()
}

pub fn print_lyrics_stdout(title: &str, artist: &str, payload: &LyricsPayload) -> Result<()> {
    println!("{title} : {artist}\n");
    match payload {
        LyricsPayload::Synced(lines) => {
            for line in lines {
                if let Some(t) = line.time_secs {
                    let m = (t / 60.0).floor() as u64;
                    let s = t % 60.0;
                    println!("[{m:02}:{s:05.2}] {}", line.text);
                } else {
                    println!("{}", line.text);
                }
            }
        }
        LyricsPayload::Plain(lines) => {
            for line in lines {
                println!("{line}");
            }
        }
        LyricsPayload::Instrumental => {
            println!("[Lagu Instrumental]");
        }
        LyricsPayload::NotFound => {
            println!("Lirik tidak ditemukan untuk lagu ini.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synced_lrc_lines_and_finds_active_index() {
        let sample = "[00:12.40] First line\n[00:16.85] Second line\n[00:21.00]\n";
        let parsed = parse_lrc(sample);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].text, "First line");
        assert_eq!(parsed[2].text, "♪");

        let payload = LyricsPayload::Synced(parsed);
        assert_eq!(payload.active_index(5.0, 200.0), None);
        assert_eq!(payload.active_index(13.0, 200.0), Some(0));
        assert_eq!(payload.active_index(17.0, 200.0), Some(1));
    }
}
