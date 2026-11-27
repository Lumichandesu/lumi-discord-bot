//! Turns user input (search text or a link) into a playable track.
//!
//! YouTube, SoundCloud and other links are handed to yt-dlp directly. Spotify, Apple Music and
//! TIDAL links are only used to read the track title and artist; the audio then comes from a
//! YouTube search for that text.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use url::Url;

const CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const CACHE_MAX: usize = 150;
const YTDLP_TIMEOUT: Duration = Duration::from_secs(60);
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    YouTube,
    SoundCloud,
    Spotify,
    AppleMusic,
    Tidal,
}

impl Provider {
    /// Providers whose links are only used to look up a title.
    pub fn is_bridge(self) -> bool {
        matches!(self, Self::Spotify | Self::AppleMusic | Self::Tidal)
    }
}

pub fn detect_provider(url: &Url) -> Option<Provider> {
    let host = url.host_str()?.to_ascii_lowercase();
    Some(match host.as_str() {
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be" | "music.youtube.com" => {
            Provider::YouTube
        }
        "soundcloud.com" | "www.soundcloud.com" | "m.soundcloud.com" | "on.soundcloud.com" => {
            Provider::SoundCloud
        }
        "open.spotify.com" | "spotify.link" => Provider::Spotify,
        "music.apple.com" | "itunes.apple.com" | "apple.co" => Provider::AppleMusic,
        "tidal.com" | "www.tidal.com" | "listen.tidal.com" => Provider::Tidal,
        _ => return None,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// Plain text: search YouTube.
    Search(String),
    /// A link yt-dlp should open directly (YouTube, SoundCloud, or any other site).
    Direct(String),
    /// A Spotify / Apple Music / TIDAL link used only to read the title.
    Bridge(String, Provider),
}

pub fn classify(input: &str) -> Target {
    let input = input.trim();
    match Url::parse(input) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => match detect_provider(&url) {
            Some(p) if p.is_bridge() => Target::Bridge(input.to_string(), p),
            _ => Target::Direct(input.to_string()),
        },
        _ => Target::Search(input.to_string()),
    }
}

/// Everything needed to display a track and open its audio stream.
#[derive(Debug, Clone)]
pub struct Track {
    pub title: String,
    pub url: String,
    pub author: String,
    /// Seconds; 0 when unknown.
    pub duration: u64,
    pub thumbnail: Option<String>,
    pub stream_url: String,
    pub protocol: Option<String>,
    pub http_headers: Vec<(String, String)>,
    pub filesize: Option<u64>,
    /// When yt-dlp produced `stream_url`. Signed stream URLs expire, so old ones are refreshed.
    pub resolved_at: Instant,
}

#[derive(Debug, Deserialize)]
struct YtDlpJson {
    title: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
    webpage_url: Option<String>,
    url: Option<String>,
    protocol: Option<String>,
    http_headers: Option<HashMap<String, String>>,
    filesize: Option<u64>,
    is_live: Option<bool>,
    live_status: Option<String>,
    entries: Option<Vec<YtDlpJson>>,
}

/// Parses the output of `yt-dlp --dump-single-json`, unwrapping the first playlist or search entry.
pub fn parse_ytdlp_json(raw: &str, fallback_url: &str) -> Result<Track, String> {
    let parsed: YtDlpJson =
        serde_json::from_str(raw).map_err(|e| format!("Could not read yt-dlp output: {e}"))?;
    let entry = match parsed.entries {
        Some(mut entries) if !entries.is_empty() => entries.swap_remove(0),
        Some(_) => return Err("No results found.".into()),
        None => parsed,
    };
    // Live and DVR streams only offer MPEG-TS over HLS, which symphonia cannot demux.
    if entry.is_live == Some(true) || matches!(entry.live_status.as_deref(), Some("is_live" | "is_upcoming" | "post_live")) {
        return Err("Live streams are not supported.".into());
    }
    let stream_url = entry.url.filter(|u| u.starts_with("http")).ok_or("No suitable audio stream found.")?;
    Ok(Track {
        title: entry.title.unwrap_or_else(|| "Unknown Title".into()),
        url: entry.webpage_url.unwrap_or_else(|| fallback_url.to_string()),
        author: entry.uploader.or(entry.channel).unwrap_or_else(|| "Unknown Artist".into()),
        duration: entry.duration.map_or(0, |d| d.max(0.0).round() as u64),
        thumbnail: entry.thumbnail,
        stream_url,
        protocol: entry.protocol,
        http_headers: entry.http_headers.map(|h| h.into_iter().collect()).unwrap_or_default(),
        filesize: entry.filesize,
        resolved_at: Instant::now(),
    })
}

fn strip_query(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// Apple Music track id: the `i=` query parameter, or a numeric last path segment.
pub fn apple_music_id(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let is_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if let Some((_, v)) = parsed.query_pairs().find(|(k, v)| k == "i" && is_digits(v)) {
        return Some(v.into_owned());
    }
    let last = parsed.path_segments()?.rev().find(|s| !s.is_empty())?;
    is_digits(last).then(|| last.to_string())
}

/// Readable name from an Apple Music path like `/us/song/some-title/123`.
pub fn apple_music_slug(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let segs: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
    segs.windows(3).find_map(|w| {
        let kind = w[0].to_ascii_lowercase();
        let numeric = w[2].bytes().all(|b| b.is_ascii_digit());
        ((kind == "song" || kind == "album") && numeric).then(|| w[1].replace('-', " ").trim().to_string())
    })
}

fn decode_entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Value of a quoted attribute inside one HTML tag.
fn tag_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{name}=");
    let mut from = 0;
    while let Some(pos) = lower[from..].find(&needle) {
        let start = from + pos;
        let preceded_ok = start == 0 || lower.as_bytes()[start - 1].is_ascii_whitespace();
        let value_start = start + needle.len();
        if preceded_ok
            && let Some(quote @ (b'"' | b'\'')) = tag.as_bytes().get(value_start).copied()
        {
            let rest = &tag[value_start + 1..];
            return rest.find(quote as char).map(|end| &rest[..end]);
        }
        from = value_start;
    }
    None
}

/// Content of `<meta property="..." content="...">` (attribute order does not matter).
pub fn meta_content(html: &str, property: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("<meta") {
        let start = from + pos;
        let end = lower[start..].find('>').map_or(html.len(), |e| start + e);
        let tag = &html[start..end];
        if tag_attr(tag, "property").is_some_and(|p| p.eq_ignore_ascii_case(property))
            && let Some(content) = tag_attr(tag, "content")
        {
            let content = decode_entities(content.trim());
            if !content.is_empty() {
                return Some(content);
            }
        }
        from = end;
    }
    None
}

fn title_tag(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = start + lower[start..].find("</title>")?;
    Some(decode_entities(html[start..end].trim()))
}

/// Removes a trailing `| TIDAL ...` or `- TIDAL ...` from a page title.
pub fn clean_tidal_title(title: &str) -> String {
    let lower = title.to_ascii_lowercase();
    let mut cut = title.len();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("tidal") {
        let at = from + pos;
        let before = lower[..at].trim_end();
        if before.ends_with('|') || before.ends_with('-') {
            cut = cut.min(before.len() - 1);
            break;
        }
        from = at + 5;
    }
    title[..cut].trim().to_string()
}

/// Artist from a TIDAL description such as `Listen to Song by Artist on TIDAL`.
pub fn tidal_artist(desc: &str) -> Option<String> {
    let lower = desc.to_ascii_lowercase();
    let start = [" by ", " from "]
        .iter()
        .filter_map(|k| lower.find(k).map(|p| p + k.len()))
        .min()
        .or_else(|| ["by ", "from "].iter().find(|k| lower.starts_with(**k)).map(|k| k.len()))?;
    let mut end = desc.len();
    for stop in [",", ".", " on tidal"] {
        if let Some(p) = lower[start..].find(stop) {
            end = end.min(start + p);
        }
    }
    let artist = desc[start..end].trim();
    (!artist.is_empty()).then(|| artist.to_string())
}

/// Artist from an Apple Music description such as `Song · Artist · 2020`.
pub fn apple_artist(desc: &str) -> Option<String> {
    let first = desc.split(['·', '•']).next()?.trim();
    (!first.is_empty()).then(|| first.to_string())
}

fn join_title(title: &str, artist: Option<&str>) -> String {
    match artist {
        Some(a) if !a.is_empty() => format!("{} {}", title.trim(), a.trim()),
        _ => title.trim().to_string(),
    }
}

pub struct Resolver {
    http: reqwest::Client,
    ytdlp: String,
    cache: Mutex<HashMap<String, (Instant, Track)>>,
    /// yt-dlp runs one at a time, like the old bot, to keep CPU spikes low on small machines.
    gate: tokio::sync::Mutex<()>,
}

impl Resolver {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http, ytdlp: find_ytdlp(), cache: Mutex::new(HashMap::new()), gate: tokio::sync::Mutex::new(()) }
    }

    pub fn ytdlp_program(&self) -> &str {
        &self.ytdlp
    }

    fn cached(&self, key: &str) -> Option<Track> {
        let mut cache = self.cache.lock().ok()?;
        match cache.get(key) {
            Some((expires, track)) if *expires > Instant::now() => Some(track.clone()),
            Some(_) => {
                cache.remove(key);
                None
            }
            None => None,
        }
    }

    fn store(&self, key: String, track: &Track) {
        let Ok(mut cache) = self.cache.lock() else { return };
        if cache.len() >= CACHE_MAX
            && let Some(oldest) = cache.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(key, (Instant::now() + CACHE_TTL, track.clone()));
    }

    pub async fn resolve(&self, input: &str) -> Result<Track, String> {
        let key = input.trim().to_lowercase();
        if let Some(t) = self.cached(&key) {
            return Ok(t);
        }
        let _turn = self.gate.lock().await;
        if let Some(t) = self.cached(&key) {
            return Ok(t);
        }

        let target = match classify(input) {
            Target::Search(q) => format!("ytsearch1:{q}"),
            Target::Direct(url) => url,
            Target::Bridge(url, provider) => {
                let query = self.bridge_query(&url, provider).await.unwrap_or_else(|| {
                    if provider == Provider::AppleMusic {
                        apple_music_slug(&url).unwrap_or(url.clone())
                    } else {
                        url.clone()
                    }
                });
                tracing::info!("bridge link resolved to search text: {query}");
                format!("ytsearch1:{query}")
            }
        };

        let raw = self.run_ytdlp(&target).await?;
        let track = parse_ytdlp_json(&raw, &target)?;
        self.store(key, &track);
        Ok(track)
    }

    /// Runs yt-dlp again on the track's page to get a fresh stream URL (the cache is skipped).
    pub async fn refresh(&self, track: &Track) -> Result<Track, String> {
        let _turn = self.gate.lock().await;
        let raw = self.run_ytdlp(&track.url).await?;
        parse_ytdlp_json(&raw, &track.url)
    }

    async fn bridge_query(&self, url: &str, provider: Provider) -> Option<String> {
        match provider {
            Provider::Spotify => self.spotify(url).await,
            Provider::AppleMusic => self.apple_music(url).await,
            Provider::Tidal => self.tidal(url).await,
            _ => None,
        }
    }

    async fn fetch_html(&self, url: &str, timeout: Duration) -> Option<String> {
        let res = self
            .http
            .get(url)
            .header(reqwest::header::USER_AGENT, BROWSER_UA)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .timeout(timeout)
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        res.text().await.ok()
    }

    async fn spotify(&self, url: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct OEmbed {
            title: Option<String>,
            author_name: Option<String>,
        }
        let res = self
            .http
            .get("https://open.spotify.com/oembed")
            .query(&[("url", strip_query(url))])
            .timeout(Duration::from_millis(3500))
            .send()
            .await
            .ok()?;
        let data: OEmbed = res.error_for_status().ok()?.json().await.ok()?;
        let title = data.title.filter(|t| !t.is_empty())?;
        Some(join_title(&title, data.author_name.as_deref()))
    }

    async fn apple_music(&self, url: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct Lookup {
            results: Vec<LookupItem>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LookupItem {
            track_name: Option<String>,
            collection_name: Option<String>,
            artist_name: Option<String>,
        }
        if let Some(id) = apple_music_id(url) {
            let lookup = async {
                let res = self
                    .http
                    .get("https://itunes.apple.com/lookup")
                    .query(&[("id", id.as_str())])
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await
                    .ok()?;
                let data: Lookup = res.error_for_status().ok()?.json().await.ok()?;
                let item = data.results.into_iter().next()?;
                let title = item.track_name.or(item.collection_name).filter(|t| !t.is_empty())?;
                Some(join_title(&title, item.artist_name.as_deref()))
            };
            if let Some(q) = lookup.await {
                return Some(q);
            }
        }
        let html = self.fetch_html(strip_query(url), Duration::from_secs(3)).await?;
        let title = meta_content(&html, "og:title")?;
        let artist = meta_content(&html, "og:description").and_then(|d| apple_artist(&d));
        Some(join_title(&title, artist.as_deref()))
    }

    async fn tidal(&self, url: &str) -> Option<String> {
        let clean = strip_query(url);
        let clean = clean.strip_suffix("/u/").or_else(|| clean.strip_suffix("/u")).unwrap_or(clean);
        let html = self.fetch_html(clean, Duration::from_millis(3500)).await?;
        if let Some(title) = meta_content(&html, "og:title") {
            let artist = meta_content(&html, "og:description").and_then(|d| tidal_artist(&d));
            return Some(join_title(&clean_tidal_title(&title), artist.as_deref()));
        }
        title_tag(&html).map(|t| clean_tidal_title(&t)).filter(|t| !t.is_empty())
    }

    async fn run_ytdlp(&self, target: &str) -> Result<String, String> {
        let args = [
            "--dump-single-json",
            "--no-playlist",
            "--skip-download",
            "-f",
            "251/ba/b",
            "--no-warnings",
            "--no-progress",
            "--socket-timeout",
            "5",
            target,
        ];
        // Run yt-dlp at the lowest CPU priority on Unix, as the old bot did.
        let mut cmd = if cfg!(unix) {
            let mut c = tokio::process::Command::new("nice");
            c.args(["-n", "19", &self.ytdlp]);
            c
        } else {
            tokio::process::Command::new(&self.ytdlp)
        };
        cmd.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);

        let output = tokio::time::timeout(YTDLP_TIMEOUT, cmd.output())
            .await
            .map_err(|_| "yt-dlp took too long to answer.".to_string())?
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "yt-dlp is not installed or not on PATH.".to_string()
                } else {
                    format!("Could not start yt-dlp: {e}")
                }
            })?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("yt-dlp failed");
            tracing::warn!("yt-dlp failed for {target}: {line}");
            return Err(line.trim().trim_start_matches("ERROR: ").to_string());
        }
        String::from_utf8(output.stdout).map_err(|_| "yt-dlp returned invalid text.".into())
    }
}

/// `YTDLP_PATH` if it points to a file, then `~/bin/yt-dlp`, then `yt-dlp` on PATH.
fn find_ytdlp() -> String {
    if let Ok(p) = std::env::var("YTDLP_PATH")
        && Path::new(&p).is_file()
    {
        return p;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = Path::new(&home).join("bin").join("yt-dlp");
        if p.is_file() {
            return p.to_string_lossy().into_owned();
        }
    }
    "yt-dlp".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(s: &str) -> Option<Provider> {
        detect_provider(&Url::parse(s).unwrap())
    }

    #[test]
    fn detects_providers_by_host() {
        assert_eq!(provider("https://www.youtube.com/watch?v=x"), Some(Provider::YouTube));
        assert_eq!(provider("https://youtu.be/x"), Some(Provider::YouTube));
        assert_eq!(provider("https://music.youtube.com/watch?v=x"), Some(Provider::YouTube));
        assert_eq!(provider("https://soundcloud.com/a/b"), Some(Provider::SoundCloud));
        assert_eq!(provider("https://open.spotify.com/track/abc"), Some(Provider::Spotify));
        assert_eq!(provider("https://music.apple.com/us/song/x/1"), Some(Provider::AppleMusic));
        assert_eq!(provider("https://listen.tidal.com/track/1"), Some(Provider::Tidal));
        assert_eq!(provider("https://TIDAL.com/browse/track/1"), Some(Provider::Tidal));
        assert_eq!(provider("https://example.com/a.mp3"), None);
        assert_eq!(provider("https://notyoutube.com/watch"), None);
    }

    #[test]
    fn classifies_input() {
        assert_eq!(classify("  never gonna give you up "), Target::Search("never gonna give you up".into()));
        assert_eq!(classify("https://youtu.be/dQw4w9WgXcQ"), Target::Direct("https://youtu.be/dQw4w9WgXcQ".into()));
        assert_eq!(classify("https://example.com/x.mp3"), Target::Direct("https://example.com/x.mp3".into()));
        assert_eq!(
            classify("https://open.spotify.com/track/abc?si=1"),
            Target::Bridge("https://open.spotify.com/track/abc?si=1".into(), Provider::Spotify)
        );
        assert!(matches!(classify("https://tidal.com/track/9"), Target::Bridge(_, Provider::Tidal)));
        assert!(matches!(classify("https://music.apple.com/us/album/a/1?i=2"), Target::Bridge(_, Provider::AppleMusic)));
        // Not http(s): treated as search text.
        assert!(matches!(classify("ftp://example.com/song"), Target::Search(_)));
        assert!(matches!(classify("artist: song"), Target::Search(_)));
    }

    #[test]
    fn apple_ids_and_slugs() {
        assert_eq!(apple_music_id("https://music.apple.com/us/album/name/123?i=456").as_deref(), Some("456"));
        assert_eq!(apple_music_id("https://music.apple.com/us/song/name/789").as_deref(), Some("789"));
        assert_eq!(apple_music_id("https://music.apple.com/us/browse"), None);
        assert_eq!(
            apple_music_slug("https://music.apple.com/us/song/never-gonna-give-you-up/1559523359").as_deref(),
            Some("never gonna give you up")
        );
        assert_eq!(apple_music_slug("https://music.apple.com/us/browse"), None);
    }

    #[test]
    fn reads_meta_tags_in_any_order() {
        let html = r#"<head><meta name="x" content="y"><META content="Song &amp; Title" property="og:title" />
            <meta property='og:description' content='Song · Artist · 2020'></head>"#;
        assert_eq!(meta_content(html, "og:title").as_deref(), Some("Song & Title"));
        assert_eq!(meta_content(html, "og:description").as_deref(), Some("Song · Artist · 2020"));
        assert_eq!(meta_content(html, "og:image"), None);
        assert_eq!(title_tag("<html><TITLE> Track - TIDAL </title>").as_deref(), Some("Track - TIDAL"));
    }

    #[test]
    fn tidal_and_apple_text_cleanup() {
        assert_eq!(clean_tidal_title("Song Name | TIDAL"), "Song Name");
        assert_eq!(clean_tidal_title("Song Name - Tidal Music"), "Song Name");
        assert_eq!(clean_tidal_title("Tidal Wave"), "Tidal Wave");
        assert_eq!(tidal_artist("Listen to Song by The Artist on TIDAL").as_deref(), Some("The Artist"));
        assert_eq!(tidal_artist("Track from Someone, 2020.").as_deref(), Some("Someone"));
        assert_eq!(tidal_artist("No artist here"), None);
        assert_eq!(apple_artist("Artist · Song · 2021").as_deref(), Some("Artist"));
    }

    #[test]
    fn parses_ytdlp_search_output() {
        let raw = r#"{"_type":"playlist","entries":[{"title":"T","uploader":"U","duration":213.4,
            "thumbnail":"https://i/x.jpg","webpage_url":"https://www.youtube.com/watch?v=a",
            "url":"https://rr.googlevideo.com/x","protocol":"https","filesize":10,
            "http_headers":{"User-Agent":"ua"}}]}"#;
        let t = parse_ytdlp_json(raw, "ytsearch1:x").unwrap();
        assert_eq!(t.title, "T");
        assert_eq!(t.author, "U");
        assert_eq!(t.duration, 213);
        assert_eq!(t.url, "https://www.youtube.com/watch?v=a");
        assert_eq!(t.protocol.as_deref(), Some("https"));
        assert_eq!(t.http_headers, vec![("User-Agent".to_string(), "ua".to_string())]);
    }

    #[test]
    fn parses_ytdlp_single_video_and_errors() {
        let t = parse_ytdlp_json(r#"{"channel":"C","url":"https://a/b"}"#, "https://src").unwrap();
        assert_eq!(t.title, "Unknown Title");
        assert_eq!(t.author, "C");
        assert_eq!(t.url, "https://src");
        assert_eq!(t.duration, 0);
        assert!(parse_ytdlp_json(r#"{"entries":[]}"#, "x").is_err());
        assert!(parse_ytdlp_json(r#"{"title":"no url"}"#, "x").is_err());
        assert!(parse_ytdlp_json("not json", "x").is_err());
        for live in [r#"{"url":"https://a/b.m3u8","is_live":true}"#, r#"{"url":"https://a/b","live_status":"is_upcoming"}"#] {
            assert_eq!(parse_ytdlp_json(live, "x").unwrap_err(), "Live streams are not supported.");
        }
        assert!(parse_ytdlp_json(r#"{"url":"https://a/b","live_status":"was_live"}"#, "x").is_ok());
    }
}
