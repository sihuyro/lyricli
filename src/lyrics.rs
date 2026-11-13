use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::Result;
use flate2::read::ZlibDecoder;
use serde::{Deserialize, Serialize};

use crate::player::{TrackState, fnv1a_64, sanitize_filename};

const KUGOU_KRC_KEY: [u8; 16] = [
    64, 71, 97, 119, 94, 50, 116, 71, 81, 54, 49, 45, 206, 210, 110, 105,
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WordTiming {
    pub start_secs: f64,
    pub end_secs: f64,
    pub char_end: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LyricLine {
    pub time_secs: Option<f64>,
    #[serde(default)]
    pub end_secs: Option<f64>,
    #[serde(default)]
    pub words: Vec<WordTiming>,
    pub text: String,
    #[serde(default)]
    pub roma: Option<String>,
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
    track_name: Option<String>,
    artist_name: Option<String>,
    instrumental: Option<bool>,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
    duration: Option<f64>,
}

#[derive(Deserialize)]
struct NetEaseSearchResp {
    result: Option<NetEaseSearchResult>,
}

#[derive(Deserialize)]
struct NetEaseSearchResult {
    songs: Option<Vec<NetEaseSong>>,
}

#[derive(Deserialize)]
struct NetEaseSong {
    id: u64,
    name: String,
    #[serde(default)]
    dt: f64,
    #[serde(default)]
    ar: Vec<NetEaseArtist>,
}

#[derive(Deserialize)]
struct NetEaseArtist {
    name: String,
}

#[derive(Deserialize)]
struct NetEaseLyricResp {
    lrc: Option<NetEaseLyricBlock>,
    yrc: Option<NetEaseLyricBlock>,
    romalrc: Option<NetEaseLyricBlock>,
}

#[derive(Deserialize)]
struct NetEaseLyricBlock {
    lyric: Option<String>,
}

#[derive(Deserialize)]
struct KugouSearchResp {
    #[serde(default)]
    candidates: Vec<KugouCandidate>,
}

#[derive(Deserialize)]
struct KugouCandidate {
    id: String,
    accesskey: String,
    #[serde(default)]
    singer: String,
    #[serde(default)]
    song: String,
    #[serde(default)]
    duration: f64,
}

#[derive(Deserialize)]
struct KugouDownloadResp {
    content: Option<String>,
}

#[derive(Deserialize)]
struct LyricsOvhResp {
    lyrics: Option<String>,
}

struct ScoredCandidate {
    payload: LyricsPayload,
    score: i32,
    roma_lines: Option<Vec<LyricLine>>,
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
                        if pos_secs + 0.02 >= t {
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
                let line = &lines[idx];
                if !line.words.is_empty() {
                    return (Some(idx), compute_word_progress(line, pos_secs + 0.02));
                }

                let start_t = line.time_secs.unwrap_or(0.0);
                if let Some(end_t) = line.end_secs
                    && end_t > start_t + 0.05
                {
                    let span = (end_t - start_t).max(0.25);
                    let prog = ((pos_secs + 0.02 - start_t) / span).clamp(0.0, 1.0);
                    return (Some(idx), prog);
                }

                let next_t = lines
                    .get(idx + 1)
                    .and_then(|l| l.time_secs)
                    .unwrap_or_else(|| (start_t + 4.5).min(total_duration.max(start_t + 4.5)));
                let raw_gap = (next_t - start_t).max(0.35);
                let line_chars = line
                    .roma
                    .as_deref()
                    .unwrap_or(&line.text)
                    .chars()
                    .count();
                let est_sung = ((line_chars as f64) * 0.13).clamp(1.6, 8.5);
                let span = if raw_gap > est_sung * 1.45 && raw_gap > 4.0 {
                    (est_sung * 1.25).min(raw_gap * 0.90)
                } else {
                    (raw_gap * 0.92).max(0.30)
                };
                let prog = ((pos_secs + 0.02 - start_t) / span).clamp(0.0, 1.0);
                (Some(idx), prog)
            }
            Self::Plain(_) => (Some(idx), 1.0),
            _ => (None, 0.0),
        }
    }
}

fn compute_word_progress(line: &LyricLine, pos_secs: f64) -> f64 {
    let total_chars = line.text.chars().count().max(1);
    let words = &line.words;
    if let Some(first) = words.first()
        && pos_secs <= first.start_secs
    {
        return 0.0;
    }
    if let Some(last) = words.last()
        && pos_secs >= last.end_secs
    {
        return 1.0;
    }

    let mut prev_char_end = 0usize;
    for w in words {
        if pos_secs < w.start_secs {
            return (prev_char_end as f64 / total_chars as f64).clamp(0.0, 1.0);
        }
        if pos_secs <= w.end_secs {
            let dur = (w.end_secs - w.start_secs).max(0.04);
            let frac = ((pos_secs - w.start_secs) / dur).clamp(0.0, 1.0);
            let exact_chars = prev_char_end as f64
                + frac * (w.char_end.saturating_sub(prev_char_end) as f64);
            return (exact_chars / total_chars as f64).clamp(0.0, 1.0);
        }
        prev_char_end = w.char_end;
    }
    1.0
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

pub fn load_track_offset(cache_key: &str) -> i64 {
    let path = cache_dir().join("offsets.json");
    let Ok(bytes) = fs::read(path) else {
        return 0;
    };
    let Ok(map) = serde_json::from_slice::<std::collections::HashMap<String, i64>>(&bytes) else {
        return 0;
    };
    map.get(cache_key).copied().unwrap_or(0)
}

pub fn save_track_offset(cache_key: &str, offset_ms: i64) {
    let path = cache_dir().join("offsets.json");
    let mut map: std::collections::HashMap<String, i64> = fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if offset_ms == 0 {
        map.remove(cache_key);
    } else {
        map.insert(cache_key.to_string(), offset_ms);
    }
    if let Ok(json) = serde_json::to_vec_pretty(&map) {
        let _ = fs::write(path, json);
    }
}

pub fn fetch_lyrics_for_track(track: &TrackState, use_cache: bool) -> LyricsPayload {
    let key = track.cache_key();
    let cache_path = cache_dir().join("lyrics").join(format!("{key}.json"));

    if use_cache
        && let Ok(cached_bytes) = fs::read(&cache_path)
        && let Ok(mut payload) = serde_json::from_slice::<LyricsPayload>(&cached_bytes)
        && !matches!(payload, LyricsPayload::NotFound)
    {
        attach_auto_romanization(&mut payload);
        return payload;
    }

    let mut payload =
        query_multi_source(&track.artist, &track.title, &track.album, track.duration_secs);
    attach_auto_romanization(&mut payload);
    if !matches!(payload, LyricsPayload::NotFound)
        && let Ok(json) = serde_json::to_vec(&payload)
    {
        let _ = fs::write(&cache_path, json);
    }
    payload
}

pub fn fetch_lyrics_by_query(artist: Option<&str>, title: &str, use_cache: bool) -> LyricsPayload {
    let artist_str = artist.unwrap_or("");
    let raw = format!("search_{artist_str}_{title}");
    let key = format!("{}_{:016x}", sanitize_filename(&raw), fnv1a_64(raw.as_bytes()));
    let cache_path = cache_dir().join("lyrics").join(format!("{key}.json"));

    if use_cache
        && let Ok(cached_bytes) = fs::read(&cache_path)
        && let Ok(mut payload) = serde_json::from_slice::<LyricsPayload>(&cached_bytes)
        && !matches!(payload, LyricsPayload::NotFound)
    {
        attach_auto_romanization(&mut payload);
        return payload;
    }

    let mut payload = query_multi_source(artist_str, title, "", 0.0);
    attach_auto_romanization(&mut payload);
    if !matches!(payload, LyricsPayload::NotFound)
        && let Ok(json) = serde_json::to_vec(&payload)
    {
        let _ = fs::write(&cache_path, json);
    }
    payload
}

fn finalize_with_roma(mut payload: LyricsPayload, extra_roma: Option<&[LyricLine]>) -> LyricsPayload {
    if let (LyricsPayload::Synced(lines), Some(roma)) = (&mut payload, extra_roma) {
        attach_roma_lines(lines, roma);
    }
    attach_auto_romanization(&mut payload);
    payload
}

fn query_multi_source(artist: &str, title: &str, album: &str, duration_secs: f64) -> LyricsPayload {
    // Tier 1 (parallel): LRCLIB + NetEase (word-synced YRC / clean LRC + romalrc).
    let (lrclib_cand, netease_cand) = std::thread::scope(|s| {
        let h_lrc = s.spawn(|| {
            query_lrclib_family("https://lrclib.net", artist, title, album, duration_secs, 15)
        });
        let h_ne = s.spawn(|| query_netease(artist, title, duration_secs));
        (h_lrc.join().ok().flatten(), h_ne.join().ok().flatten())
    });

    let netease_roma = netease_cand.as_ref().and_then(|c| c.roma_lines.clone());
    let mut best_tier1 = pick_best_candidate(lrclib_cand, netease_cand);
    if let Some(best) = best_tier1.as_ref()
        && matches!(best.payload, LyricsPayload::Synced(_) | LyricsPayload::Instrumental)
    {
        return finalize_with_roma(best.payload.clone(), netease_roma.as_deref());
    }

    // Tier 2 (parallel): KuGou (word-synced KRC / LRC) + Karalyr fallback.
    let (kugou_cand, karalyr_cand) = std::thread::scope(|s| {
        let h_kg = s.spawn(|| query_kugou(artist, title, duration_secs));
        let h_kr = s.spawn(|| {
            query_lrclib_family("https://karalyr.com", artist, title, album, duration_secs, 0)
        });
        (h_kg.join().ok().flatten(), h_kr.join().ok().flatten())
    });

    let best_tier2 = pick_best_candidate(kugou_cand, karalyr_cand);
    best_tier1 = pick_best_candidate(best_tier1, best_tier2);
    if let Some(best) = best_tier1.as_ref()
        && matches!(best.payload, LyricsPayload::Synced(_) | LyricsPayload::Instrumental)
    {
        return finalize_with_roma(best.payload.clone(), netease_roma.as_deref());
    }

    // Tier 3: Plain lyrics fallback (already found plain or query lyrics.ovh).
    if let Some(best) = best_tier1
        && !matches!(best.payload, LyricsPayload::NotFound)
    {
        return best.payload;
    }

    if !artist.is_empty()
        && !title.is_empty()
        && let Some(plain) = query_lyrics_ovh(artist, title)
    {
        return plain;
    }

    LyricsPayload::NotFound
}

fn pick_best_candidate(
    a: Option<ScoredCandidate>,
    b: Option<ScoredCandidate>,
) -> Option<ScoredCandidate> {
    match (a, b) {
        (Some(ca), Some(cb)) => {
            if cb.score > ca.score {
                Some(cb)
            } else {
                Some(ca)
            }
        }
        (Some(ca), None) => Some(ca),
        (None, Some(cb)) => Some(cb),
        (None, None) => None,
    }
}

fn payload_quality_score(payload: &LyricsPayload) -> i32 {
    match payload {
        LyricsPayload::Synced(lines) => {
            let has_words = lines.iter().any(|l| !l.words.is_empty());
            if has_words { 260 } else { 180 }
        }
        LyricsPayload::Instrumental => 120,
        LyricsPayload::Plain(_) => 45,
        LyricsPayload::NotFound => -1000,
    }
}

fn query_lrclib_family(
    base_url: &str,
    artist: &str,
    title: &str,
    album: &str,
    duration_secs: f64,
    provider_bias: i32,
) -> Option<ScoredCandidate> {
    let clean_title = strip_track_noise(title);
    let primary_artist = primary_artist_name(artist);

    if !artist.is_empty() && !title.is_empty() {
        for (a, t, alb) in [
            (artist, title, album),
            (primary_artist, clean_title.as_str(), ""),
        ] {
            if a.is_empty() || t.is_empty() {
                continue;
            }
            if let Some(resp) = lrclib_get_exact(base_url, a, t, alb, duration_secs) {
                let payload = convert_lrclib_response(resp);
                if !matches!(payload, LyricsPayload::NotFound) {
                    let score = payload_quality_score(&payload) + 95 + provider_bias;
                    return Some(ScoredCandidate {
                        payload,
                        score,
                        roma_lines: None,
                    });
                }
            }
        }
    }

    let queries = [
        format!("{primary_artist} {clean_title}").trim().to_string(),
        format!("{primary_artist} {title}").trim().to_string(),
        clean_title,
    ];

    let mut best: Option<ScoredCandidate> = None;
    for q in queries {
        if q.is_empty() {
            continue;
        }
        if let Some((resp, meta_score)) = lrclib_search(base_url, &q, artist, title, duration_secs) {
            let payload = convert_lrclib_response(resp);
            if !matches!(payload, LyricsPayload::NotFound) {
                let score = payload_quality_score(&payload) + meta_score + provider_bias;
                let cand = ScoredCandidate {
                    payload,
                    score,
                    roma_lines: None,
                };
                let is_synced = matches!(
                    cand.payload,
                    LyricsPayload::Synced(_) | LyricsPayload::Instrumental
                );
                best = pick_best_candidate(best, Some(cand));
                if is_synced {
                    return best;
                }
            }
        }
    }

    best
}

fn lrclib_get_exact(
    base_url: &str,
    artist: &str,
    title: &str,
    album: &str,
    duration_secs: f64,
) -> Option<LrclibResponse> {
    let url = format!("{base_url}/api/get");
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sSL",
        "--max-time",
        "5",
        "-A",
        "lyricli/0.1.0 (https://github.com/sihuyro/lyricli)",
        "-G",
        &url,
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

fn lrclib_search(
    base_url: &str,
    query: &str,
    target_artist: &str,
    target_title: &str,
    target_duration: f64,
) -> Option<(LrclibResponse, i32)> {
    let url = format!("{base_url}/api/search");
    let out = Command::new("curl")
        .args([
            "-sSL",
            "--max-time",
            "5",
            "-A",
            "lyricli/0.1.0 (https://github.com/sihuyro/lyricli)",
            "-G",
            &url,
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

    items
        .into_iter()
        .filter_map(|item| {
            let cand_title = item.track_name.as_deref().unwrap_or("");
            let cand_artist = item.artist_name.as_deref().unwrap_or("");
            let meta_score = score_metadata(
                target_artist,
                target_title,
                target_duration,
                cand_artist,
                cand_title,
                item.duration.unwrap_or(0.0),
            )?;
            let lyric_bonus = if item.synced_lyrics.as_ref().is_some_and(|s| !s.trim().is_empty()) {
                100
            } else if item.plain_lyrics.as_ref().is_some_and(|s| !s.trim().is_empty()) {
                35
            } else if item.instrumental == Some(true) {
                60
            } else {
                return None;
            };
            Some((item, meta_score, meta_score + lyric_bonus))
        })
        .max_by_key(|(_, _, total)| *total)
        .map(|(item, meta_score, _)| (item, meta_score))
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
        let lines: Vec<String> = plain.lines().map(|l| l.trim().to_string()).collect();
        if lines.iter().any(|l| !l.is_empty()) {
            return LyricsPayload::Plain(lines);
        }
    }
    LyricsPayload::NotFound
}

fn query_netease(artist: &str, title: &str, duration_secs: f64) -> Option<ScoredCandidate> {
    let clean_title = strip_track_noise(title);
    let primary_artist = primary_artist_name(artist);
    let query = format!("{primary_artist} {clean_title}").trim().to_string();
    if query.is_empty() {
        return None;
    }

    let out = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "5",
            "-A",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36",
            "-H",
            "Referer: https://music.163.com",
            "-G",
            "https://music.163.com/api/cloudsearch/pc",
            "--data-urlencode",
        ])
        .arg(format!("s={query}"))
        .args(["--data-urlencode", "type=1", "--data-urlencode", "limit=6"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let search: NetEaseSearchResp = serde_json::from_slice(&out.stdout).ok()?;
    let songs = search.result?.songs?;
    let mut ranked: Vec<(NetEaseSong, i32)> = songs
        .into_iter()
        .filter_map(|song| {
            let cand_artist = song
                .ar
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let cand_dur = song.dt / 1000.0;
            let s = score_metadata(
                artist,
                title,
                duration_secs,
                &cand_artist,
                &song.name,
                cand_dur,
            )?;
            Some((song, s))
        })
        .collect();

    ranked.sort_by_key(|a| std::cmp::Reverse(a.1));

    for (song, meta_score) in ranked.into_iter().take(2) {
        let lyric_url = format!(
            "https://music.163.com/api/song/lyric?os=pc&id={}&lv=1&kv=1&tv=-1&yv=1&rv=1",
            song.id
        );
        let Ok(lrc_out) = Command::new("curl")
            .args([
                "-sS",
                "--max-time",
                "5",
                "-A",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36",
                "-H",
                "Referer: https://music.163.com",
                &lyric_url,
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            continue;
        };

        if !lrc_out.status.success() {
            continue;
        }
        let Ok(resp) = serde_json::from_slice::<NetEaseLyricResp>(&lrc_out.stdout) else {
            continue;
        };

        let roma_lines = resp
            .romalrc
            .and_then(|r| r.lyric)
            .map(|raw| parse_lrc(&raw))
            .filter(|v| !v.is_empty());

        if let Some(yrc_raw) = resp.yrc.and_then(|y| y.lyric) {
            if is_netease_instrumental(&yrc_raw) {
                return Some(ScoredCandidate {
                    payload: LyricsPayload::Instrumental,
                    score: 120 + meta_score,
                    roma_lines: None,
                });
            }
            let mut lines = parse_yrc(&yrc_raw);
            if !lines.is_empty() {
                if let Some(ref roma) = roma_lines {
                    attach_roma_lines(&mut lines, roma);
                }
                let payload = LyricsPayload::Synced(lines);
                let score = payload_quality_score(&payload) + meta_score;
                return Some(ScoredCandidate {
                    payload,
                    score,
                    roma_lines,
                });
            }
        }

        if let Some(lrc_raw) = resp.lrc.and_then(|l| l.lyric) {
            if is_netease_instrumental(&lrc_raw) {
                return Some(ScoredCandidate {
                    payload: LyricsPayload::Instrumental,
                    score: 120 + meta_score,
                    roma_lines: None,
                });
            }
            let mut lines = parse_lrc(&lrc_raw);
            if !lines.is_empty() {
                if let Some(ref roma) = roma_lines {
                    attach_roma_lines(&mut lines, roma);
                }
                let payload = LyricsPayload::Synced(lines);
                let score = payload_quality_score(&payload) + meta_score;
                return Some(ScoredCandidate {
                    payload,
                    score,
                    roma_lines,
                });
            }
            let plain: Vec<String> = lrc_raw
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !is_credit_line(l))
                .collect();
            if !plain.is_empty() {
                let payload = LyricsPayload::Plain(plain);
                let score = payload_quality_score(&payload) + meta_score;
                return Some(ScoredCandidate {
                    payload,
                    score,
                    roma_lines,
                });
            }
        }
    }

    None
}

fn query_kugou(artist: &str, title: &str, duration_secs: f64) -> Option<ScoredCandidate> {
    let clean_title = strip_track_noise(title);
    let primary_artist = primary_artist_name(artist);
    let keyword = if primary_artist.is_empty() {
        clean_title
    } else {
        format!("{primary_artist} - {clean_title}")
    };
    if keyword.trim().is_empty() {
        return None;
    }

    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--max-time",
        "5",
        "https://krcs.kugou.com/search",
        "-G",
        "--data-urlencode",
        "ver=1",
        "--data-urlencode",
        "man=yes",
        "--data-urlencode",
        "client=mobi",
        "--data-urlencode",
    ])
    .arg(format!("keyword={keyword}"));

    if duration_secs > 1.0 {
        cmd.arg("--data-urlencode")
            .arg(format!("duration={}", (duration_secs * 1000.0).round() as u64));
    }

    let out = cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }

    let search: KugouSearchResp = serde_json::from_slice(&out.stdout).ok()?;
    let mut ranked: Vec<(KugouCandidate, i32)> = search
        .candidates
        .into_iter()
        .filter_map(|cand| {
            let s = score_metadata(
                artist,
                title,
                duration_secs,
                &cand.singer,
                &cand.song,
                cand.duration / 1000.0,
            )?;
            Some((cand, s))
        })
        .collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.1));

    for (cand, meta_score) in ranked.into_iter().take(2) {
        if let Some(krc_text) = download_kugou_lyric(&cand.id, &cand.accesskey, "krc")
            && let Some(decoded) = decode_kugou_krc(&krc_text)
        {
            let lines = parse_krc(&decoded);
            if !lines.is_empty() {
                let payload = LyricsPayload::Synced(lines);
                let score = payload_quality_score(&payload) + meta_score;
                return Some(ScoredCandidate {
                    payload,
                    score,
                    roma_lines: None,
                });
            }
        }

        if let Some(lrc_b64) = download_kugou_lyric(&cand.id, &cand.accesskey, "lrc")
            && let Some(raw_bytes) = decode_base64(&lrc_b64)
            && let Ok(lrc_text) = String::from_utf8(raw_bytes)
        {
            let lines = parse_lrc(&lrc_text);
            if !lines.is_empty() {
                let payload = LyricsPayload::Synced(lines);
                let score = payload_quality_score(&payload) + meta_score;
                return Some(ScoredCandidate {
                    payload,
                    score,
                    roma_lines: None,
                });
            }
        }
    }

    None
}

fn download_kugou_lyric(id: &str, accesskey: &str, fmt: &str) -> Option<String> {
    let url = format!(
        "https://lyrics.kugou.com/download?ver=1&client=pc&id={id}&accesskey={accesskey}&fmt={fmt}&charset=utf8"
    );
    let out = Command::new("curl")
        .args(["-sS", "--max-time", "5", &url])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let resp: KugouDownloadResp = serde_json::from_slice(&out.stdout).ok()?;
    resp.content
}

fn decode_kugou_krc(content_b64: &str) -> Option<String> {
    let raw = decode_base64(content_b64)?;
    if raw.len() < 4 || &raw[..4] != b"krc1" {
        return None;
    }
    let xored: Vec<u8> = raw[4..]
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ KUGOU_KRC_KEY[i % KUGOU_KRC_KEY.len()])
        .collect();
    let mut decoder = ZlibDecoder::new(xored.as_slice());
    let mut out = String::new();
    decoder.read_to_string(&mut out).ok()?;
    Some(out)
}

fn query_lyrics_ovh(artist: &str, title: &str) -> Option<LyricsPayload> {
    let primary_artist = primary_artist_name(artist);
    let clean_title = strip_track_noise(title);
    if primary_artist.is_empty() || clean_title.is_empty() {
        return None;
    }
    let url = format!(
        "https://api.lyrics.ovh/v1/{}/{}",
        url_encode_segment(primary_artist),
        url_encode_segment(&clean_title)
    );
    let out = Command::new("curl")
        .args(["-sSL", "--max-time", "5", &url])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let resp: LyricsOvhResp = serde_json::from_slice(&out.stdout).ok()?;
    let raw = resp.lyrics?;
    let lines: Vec<String> = raw.lines().map(|l| l.trim().to_string()).collect();
    if lines.iter().any(|l| !l.is_empty()) {
        Some(LyricsPayload::Plain(lines))
    } else {
        None
    }
}

fn score_metadata(
    target_artist: &str,
    target_title: &str,
    target_duration: f64,
    cand_artist: &str,
    cand_title: &str,
    cand_duration: f64,
) -> Option<i32> {
    let t_title = normalize_tokens(&strip_track_noise(target_title));
    let c_title = normalize_tokens(&strip_track_noise(cand_title));
    if !t_title.is_empty() && !c_title.is_empty() {
        let title_overlap = token_overlap_ratio(&t_title, &c_title);
        if title_overlap < 0.34 {
            return None;
        }
    }

    let mut score = 0i32;

    if !t_title.is_empty() && t_title == c_title {
        score += 65;
    } else {
        score += (token_overlap_ratio(&t_title, &c_title) * 50.0) as i32;
    }

    let t_artist = normalize_tokens(primary_artist_name(target_artist));
    let c_artist = normalize_tokens(cand_artist);
    if !t_artist.is_empty() && !c_artist.is_empty() {
        let artist_overlap = token_overlap_ratio(&t_artist, &c_artist);
        if artist_overlap == 0.0 {
            score -= 60;
        } else if artist_overlap >= 0.99 {
            score += 50;
        } else {
            score += (artist_overlap * 40.0) as i32;
        }
    }

    if target_duration > 1.0 && cand_duration > 1.0 {
        let diff = (cand_duration - target_duration).abs();
        score += if diff <= 2.5 {
            80
        } else if diff <= 5.0 {
            50
        } else if diff <= 10.0 {
            15
        } else if diff <= 20.0 {
            -30
        } else {
            -120
        };
    }

    // Penalize unexpected variants (remix/live/acoustic/instrumental) when not requested.
    let target_lower = target_title.to_ascii_lowercase();
    let cand_lower = cand_title.to_ascii_lowercase();
    for tag in [
        "remix",
        "live",
        "acoustic",
        "instrumental",
        "karaoke",
        "sped up",
        "slowed",
        "cover",
    ] {
        if cand_lower.contains(tag) && !target_lower.contains(tag) {
            score -= 70;
        }
    }

    Some(score)
}

fn normalize_tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|tok| !tok.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn token_overlap_ratio(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let hits = a.iter().filter(|tok| b.contains(tok)).count();
    hits as f64 / a.len().max(1) as f64
}

fn primary_artist_name(artist: &str) -> &str {
    artist.split(',').next().unwrap_or(artist).trim()
}

fn is_netease_instrumental(raw: &str) -> bool {
    let trimmed = raw.trim();
    trimmed.contains("纯音乐，请欣赏") || trimmed.contains("纯音乐,请欣赏")
}

fn is_credit_line(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    for prefix in [
        "作词",
        "作曲",
        "编曲",
        "制作人",
        "混音",
        "母带",
        "录音",
        "歌词贡献",
        "翻译贡献",
        "lyrics by",
        "composed by",
        "arranged by",
        "written by",
        "produced by",
    ] {
        if lower.starts_with(prefix) {
            return true;
        }
    }
    false
}

pub fn parse_lrc(lrc_text: &str) -> Vec<LyricLine> {
    let mut result = Vec::new();
    let mut global_offset_secs = 0.0f64;

    for raw_line in lrc_text.lines() {
        let mut rest = raw_line.trim();
        if let Some(after_off) = rest.strip_prefix("[offset:")
            && let Some(end_off) = after_off.find(']')
            && let Ok(ms) = after_off[..end_off].trim().parse::<f64>()
        {
            global_offset_secs = ms / 1000.0;
            continue;
        }

        let mut timestamps = Vec::new();
        while let Some(after_open) = rest.strip_prefix('[') {
            let Some(close_idx) = after_open.find(']') else {
                break;
            };
            let tag = &after_open[..close_idx];
            if let Some(secs) = parse_timestamp_tag(tag) {
                timestamps.push((secs - global_offset_secs).max(0.0));
                rest = after_open[close_idx + 1..].trim_start();
            } else {
                break;
            }
        }

        if timestamps.is_empty() {
            continue;
        }

        let (clean_text, parsed_words) = parse_enhanced_lrc_words(rest, global_offset_secs);
        if clean_text.is_empty() {
            for t in timestamps {
                result.push(LyricLine {
                    time_secs: Some(t),
                    end_secs: None,
                    words: Vec::new(),
                    text: "♪".to_string(),
                    roma: None,
                });
            }
            continue;
        }

        for t in timestamps {
            result.push(LyricLine {
                time_secs: Some(t),
                end_secs: parsed_words.last().map(|w| w.end_secs),
                words: parsed_words.clone(),
                text: clean_text.clone(),
                roma: None,
            });
        }
    }

    finalize_synced_lines(result)
}

fn parse_enhanced_lrc_words(raw: &str, global_offset_secs: f64) -> (String, Vec<WordTiming>) {
    let mut clean = String::with_capacity(raw.len());
    let mut starts: Vec<(f64, usize)> = Vec::new();
    let mut rest = raw;

    while let Some(lt) = rest.find('<') {
        let before = &rest[..lt];
        clean.push_str(before);
        let after_lt = &rest[lt + 1..];
        if let Some(gt) = after_lt.find('>')
            && let Some(secs) = parse_timestamp_tag(&after_lt[..gt])
        {
            let adj = (secs - global_offset_secs).max(0.0);
            starts.push((adj, clean.chars().count()));
            rest = &after_lt[gt + 1..];
            continue;
        }
        clean.push('<');
        rest = after_lt;
    }
    clean.push_str(rest);
    let trimmed = clean.trim().to_string();

    if starts.len() < 2 {
        return (trimmed, Vec::new());
    }

    let total_chars = trimmed.chars().count();
    let leading_trim = clean.chars().take_while(|c| c.is_whitespace()).count();
    let mut words = Vec::with_capacity(starts.len());
    for (i, &(start_secs, raw_char_start)) in starts.iter().enumerate() {
        let next_start = starts
            .get(i + 1)
            .map(|(s, _)| *s)
            .unwrap_or(start_secs + 0.45);
        let next_char = starts
            .get(i + 1)
            .map(|(_, c)| c.saturating_sub(leading_trim).min(total_chars))
            .unwrap_or(total_chars);
        let char_start = raw_char_start.saturating_sub(leading_trim).min(total_chars);
        if next_char <= char_start {
            continue;
        }
        words.push(WordTiming {
            start_secs,
            end_secs: next_start.max(start_secs + 0.05),
            char_end: next_char,
        });
    }

    (trimmed, words)
}

fn parse_yrc(yrc_text: &str) -> Vec<LyricLine> {
    let mut result = Vec::new();

    for raw_line in yrc_text.lines() {
        let line = raw_line.trim();
        if !line.starts_with('[') {
            continue;
        }
        let Some(close_bracket) = line.find(']') else {
            continue;
        };
        let header = &line[1..close_bracket];
        let Some((start_ms_str, dur_ms_str)) = header.split_once(',') else {
            continue;
        };
        let (Ok(start_ms), Ok(dur_ms)) = (
            start_ms_str.trim().parse::<f64>(),
            dur_ms_str.trim().parse::<f64>(),
        ) else {
            continue;
        };

        let body = &line[close_bracket + 1..];
        let (text, words) = parse_word_tokens(body, '(', ')', 0.0);
        if text.is_empty() {
            continue;
        }

        let start_secs = (start_ms / 1000.0).max(0.0);
        let end_secs = ((start_ms + dur_ms) / 1000.0).max(start_secs + 0.1);
        result.push(LyricLine {
            time_secs: Some(start_secs),
            end_secs: Some(end_secs),
            words,
            text,
            roma: None,
        });
    }

    finalize_synced_lines(result)
}

fn parse_krc(krc_text: &str) -> Vec<LyricLine> {
    let mut result = Vec::new();
    let mut global_offset_secs = 0.0f64;

    for raw_line in krc_text.lines() {
        let line = raw_line.trim();
        if let Some(after_off) = line.strip_prefix("[offset:")
            && let Some(end_off) = after_off.find(']')
            && let Ok(ms) = after_off[..end_off].trim().parse::<f64>()
        {
            global_offset_secs = ms / 1000.0;
            continue;
        }

        if !line.starts_with('[') {
            continue;
        }
        let Some(close_bracket) = line.find(']') else {
            continue;
        };
        let header = &line[1..close_bracket];
        let Some((start_ms_str, dur_ms_str)) = header.split_once(',') else {
            continue;
        };
        let (Ok(start_ms), Ok(dur_ms)) = (
            start_ms_str.trim().parse::<f64>(),
            dur_ms_str.trim().parse::<f64>(),
        ) else {
            continue;
        };

        let start_secs = ((start_ms / 1000.0) - global_offset_secs).max(0.0);
        let end_secs = (((start_ms + dur_ms) / 1000.0) - global_offset_secs).max(start_secs + 0.1);
        let body = &line[close_bracket + 1..];
        let (text, words) = parse_word_tokens(body, '<', '>', start_secs);
        if text.is_empty() {
            continue;
        }

        result.push(LyricLine {
            time_secs: Some(start_secs),
            end_secs: Some(end_secs),
            words,
            text,
            roma: None,
        });
    }

    finalize_synced_lines(result)
}

// Parses `(start_ms,dur_ms,flag)text` (NetEase YRC) or `<rel_ms,dur_ms,flag>text` (KuGou KRC).
fn parse_word_tokens(
    body: &str,
    open_delim: char,
    close_delim: char,
    base_offset_secs: f64,
) -> (String, Vec<WordTiming>) {
    let mut tokens: Vec<(f64, f64, usize)> = Vec::new();
    let mut text = String::with_capacity(body.len());

    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = body[i..].chars().next().unwrap();
        if ch == open_delim
            && let Some(rel_close) = body[i + 1..].find(close_delim)
        {
            let tag = &body[i + 1..i + 1 + rel_close];
            let mut parts = tag.split(',');
            if let (Some(a), Some(b), Some(_c), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
                && let (Ok(w_start_ms), Ok(w_dur_ms)) =
                    (a.trim().parse::<f64>(), b.trim().parse::<f64>())
            {
                let start_secs = (base_offset_secs + w_start_ms / 1000.0).max(0.0);
                let end_secs = (start_secs + w_dur_ms / 1000.0).max(start_secs + 0.04);
                let token_start_char = text.chars().count();
                tokens.push((start_secs, end_secs, token_start_char));
                i += 1 + rel_close + 1;
                continue;
            }
        }
        text.push(ch);
        i += ch.len_utf8();
    }

    let trimmed = text.trim().to_string();
    let leading_ws = text.chars().take_while(|c| c.is_whitespace()).count();
    let total_chars = trimmed.chars().count();

    let mut words = Vec::with_capacity(tokens.len());
    for idx in 0..tokens.len() {
        let (start_secs, end_secs, _) = tokens[idx];
        let next_start_char = tokens
            .get(idx + 1)
            .map(|(_, _, c)| c.saturating_sub(leading_ws).min(total_chars))
            .unwrap_or(total_chars);
        let cur_start_char = tokens[idx].2.saturating_sub(leading_ws).min(total_chars);
        if next_start_char <= cur_start_char {
            continue;
        }
        words.push(WordTiming {
            start_secs,
            end_secs,
            char_end: next_start_char,
        });
    }

    (trimmed, words)
}

fn finalize_synced_lines(mut result: Vec<LyricLine>) -> Vec<LyricLine> {
    result.sort_by(|a, b| {
        a.time_secs
            .unwrap_or(0.0)
            .partial_cmp(&b.time_secs.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Drop leading NetEase/KuGou composer credit lines if real lyrics follow.
    let real_count = result
        .iter()
        .filter(|l| l.text != "♪" && !is_credit_line(&l.text))
        .count();
    if real_count > 0 {
        result.retain(|l| l.time_secs.unwrap_or(0.0) >= 15.0 || !is_credit_line(&l.text));
    }

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
                    || inner.contains("with ")
                    || inner.contains("version")
                    || inner.contains("live")
                    || inner.contains("edit")
                    || inner.contains("explicit")
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

fn url_encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity((input.len() * 3) / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;

    for b in input.bytes() {
        if b.is_ascii_whitespace() || b == b'=' {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
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

fn attach_roma_lines(lines: &mut [LyricLine], roma_lines: &[LyricLine]) {
    if roma_lines.is_empty() {
        return;
    }

    for line in lines.iter_mut() {
        if line.roma.is_some() || !has_non_ascii_letters(&line.text) {
            continue;
        }
        let Some(t) = line.time_secs else {
            continue;
        };

        let best = roma_lines
            .iter()
            .filter_map(|r| {
                let rt = r.time_secs?;
                let diff = (rt - t).abs();
                let clean = r.text.trim();
                if diff <= 0.65 && !clean.is_empty() && clean != "♪" {
                    Some((diff, clean))
                } else {
                    None
                }
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));

        if let Some((_, matched)) = best {
            line.roma = Some(matched.to_string());
        }
    }
}

fn attach_auto_romanization(payload: &mut LyricsPayload) {
    let LyricsPayload::Synced(lines) = payload else {
        return;
    };
    for line in lines.iter_mut() {
        if line.roma.is_some() {
            continue;
        }
        if let Some(roma) = romanize_hangul(&line.text) {
            line.roma = Some(roma);
        }
    }
}

fn has_non_ascii_letters(text: &str) -> bool {
    text.chars().any(|c| !c.is_ascii() && c.is_alphabetic())
}

// Revised Romanization of Hangul syllables (U+AC00..=U+D7A3) as offline fallback.
pub fn romanize_hangul(text: &str) -> Option<String> {
    const INITIALS: [&str; 19] = [
        "g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t",
        "p", "h",
    ];
    const MEDIALS: [&str; 21] = [
        "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo",
        "we", "wi", "yu", "eu", "ui", "i",
    ];
    const FINALS: [&str; 28] = [
        "", "k", "kk", "k", "n", "n", "n", "t", "l", "lk", "lm", "lp", "l", "l", "l", "l", "m",
        "p", "p", "t", "t", "ng", "t", "t", "k", "t", "p", "t",
    ];

    let mut out = String::with_capacity(text.len() * 2);
    let mut converted_any = false;

    for ch in text.chars() {
        let code = ch as u32;
        if (0xAC00..=0xD7A3).contains(&code) {
            let s_idx = (code - 0xAC00) as usize;
            let init = s_idx / 588;
            let med = (s_idx % 588) / 28;
            let fin = s_idx % 28;
            out.push_str(INITIALS[init]);
            out.push_str(MEDIALS[med]);
            out.push_str(FINALS[fin]);
            converted_any = true;
        } else {
            out.push(ch);
        }
    }

    if converted_any { Some(out) } else { None }
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

    #[test]
    fn parses_word_level_yrc_and_krc_timestamps() {
        let yrc = "[1000,2000](1000,500,0)Hello (1500,1500,0)world\n";
        let parsed_yrc = parse_yrc(yrc);
        assert_eq!(parsed_yrc.len(), 1);
        assert_eq!(parsed_yrc[0].text, "Hello world");
        assert_eq!(parsed_yrc[0].words.len(), 2);

        let payload = LyricsPayload::Synced(parsed_yrc);
        let (_, p_mid) = payload.active_index_and_progress(2.25, 200.0);
        assert!(p_mid > 0.4 && p_mid < 0.9);

        let krc = "[offset:0]\n[1000,2000]<0,500,0>Hello <500,1500,0>world\n";
        let parsed_krc = parse_krc(krc);
        assert_eq!(parsed_krc.len(), 1);
        assert_eq!(parsed_krc[0].words.len(), 2);
        assert!((parsed_krc[0].words[1].start_secs - 1.5).abs() < 0.01);
    }

    #[test]
    fn parses_enhanced_lrc_inline_word_tags() {
        let sample = "[00:10.00] <00:10.00>Hello <00:11.00>world\n";
        let parsed = parse_lrc(sample);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].text, "Hello world");
        assert_eq!(parsed[0].words.len(), 2);
    }

    #[test]
    fn romanizes_hangul_syllables_and_attaches_romalrc() {
        assert_eq!(
            romanize_hangul("사랑해 Hurt").as_deref(),
            Some("saranghae Hurt")
        );
        assert_eq!(romanize_hangul("English only"), None);

        let mut base = parse_lrc("[00:10.00] 夜に浮かんでいた\n[00:14.00] Yeah\n");
        let roma = parse_lrc("[00:10.05] yo ru ni u ka n de i ta\n[00:14.00] Yeah\n");
        attach_roma_lines(&mut base, &roma);
        assert_eq!(base[0].roma.as_deref(), Some("yo ru ni u ka n de i ta"));
        assert_eq!(base[1].roma, None);
    }
}
