//! Turns user input (search text or a link) into a playable track.
//!
//! YouTube, SoundCloud and other links are handed to yt-dlp directly. Spotify, Apple Music and
//! TIDAL links are only used to read the track title and artist; the audio then comes from a
//! YouTube search for that text.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use url::{Host, Url};

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
        "apple.co" => Provider::AppleMusic,
        // Bridge hosts also match subdomains (geo.music.apple.com, embed.tidal.com), as the old bot did.
        h if ["music.apple.com", "itunes.apple.com"].iter().any(|d| is_domain_or_sub(h, d)) => Provider::AppleMusic,
        h if is_domain_or_sub(h, "tidal.com") => Provider::Tidal,
        _ => return None,
    })
}

fn is_domain_or_sub(host: &str, domain: &str) -> bool {
    host.strip_suffix(domain).is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
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

/// Addresses the bot must never fetch for a user: loopback, private, link-local (cloud metadata),
/// carrier-grade NAT, unique-local, multicast and unspecified.
pub fn is_internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || (a == 100 && (b & 0xc0) == 64)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_internal_ip(v4.into()),
            None => {
                let first = v6.segments()[0];
                v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
            }
        },
    }
}

/// Rejects links to hosts on the bot's own network. YouTube and SoundCloud links skip the DNS lookup.
async fn ensure_public(url: &str) -> Result<(), String> {
    let blocked = || "Links to private or local network addresses are not allowed.".to_string();
    let parsed = Url::parse(url).map_err(|_| "Invalid link.".to_string())?;
    if detect_provider(&parsed).is_some() {
        return Ok(());
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    let ips: Vec<IpAddr> = match parsed.host() {
        Some(Host::Ipv4(ip)) => vec![ip.into()],
        Some(Host::Ipv6(ip)) => vec![ip.into()],
        Some(Host::Domain(d)) => tokio::net::lookup_host((d, port))
            .await
            .map_err(|_| "Could not resolve the link's host.".to_string())?
            .map(|a| a.ip())
            .collect(),
        None => return Err("Invalid link.".into()),
    };
    if ips.is_empty() || ips.into_iter().any(is_internal_ip) { Err(blocked()) } else { Ok(()) }
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
    /// Platform label shown on the now-playing embed (`YOUTUBE`, `SOUNDCLOUD`).
    pub platform: &'static str,
    /// Who queued the track; set by the player, not by the resolver.
    pub requester: Option<serenity::all::UserId>,
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
        platform: "YOUTUBE",
        requester: None,
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
        // Requests wait in line for yt-dlp as long as needed, like the old bot's queue.
        let _turn = self.gate.lock().await;
        if let Some(t) = self.cached(&key) {
            return Ok(t);
        }

        let mut direct = false;
        let target = match classify(input) {
            Target::Search(q) => format!("ytsearch1:{q}"),
            Target::Direct(url) => {
                ensure_public(&url).await?;
                direct = true;
                url
            }
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
        let mut track = parse_ytdlp_json(&raw, &target)?;
        if direct {
            // Direct links show their site; searches and bridge links play from YouTube.
            if Url::parse(&target).ok().and_then(|u| detect_provider(&u)) == Some(Provider::SoundCloud) {
                track.platform = "SOUNDCLOUD";
            }
            // A generic page can point yt-dlp at media on an internal host.
            ensure_public(&track.stream_url).await?;
        }
        self.store(key, &track);
        Ok(track)
    }

    /// Runs yt-dlp again on the track's page to get a fresh stream URL (the cache is skipped).
    pub async fn refresh(&self, track: &Track) -> Result<Track, String> {
        ensure_public(&track.url).await?;
        let _turn = self.gate.lock().await;
        let raw = self.run_ytdlp(&track.url).await?;
        let mut fresh = parse_ytdlp_json(&raw, &track.url)?;
        ensure_public(&fresh.stream_url).await?;
        fresh.platform = track.platform;
        fresh.requester = track.requester;
        Ok(fresh)
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
            // A playlist-only link would otherwise resolve every entry and hold the gate for a minute.
            "--playlist-items",
            "1",
            "--skip-download",
            "-f",
            // Live streams have no audio-only format; a 360p one keeps 128 kbps audio
            // without pulling a full HD video stream.
            "251/ba/b[height<=360]/b",
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
        assert_eq!(provider("https://geo.music.apple.com/us/album/x/123?i=456"), Some(Provider::AppleMusic));
        assert_eq!(provider("https://embed.music.apple.com/us/album/x/1"), Some(Provider::AppleMusic));
        assert_eq!(provider("https://embed.tidal.com/tracks/1"), Some(Provider::Tidal));
        assert_eq!(provider("https://nottidal.com/track/1"), None);
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
    fn internal_addresses_are_blocked() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1"] {
            assert!(is_internal_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "142.250.1.1", "2001:4860:4860::8888"] {
            assert!(!is_internal_ip(ip.parse().unwrap()), "{ip}");
        }
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
        let live = parse_ytdlp_json(r#"{"url":"https://a/b.m3u8","protocol":"m3u8_native","is_live":true}"#, "x").unwrap();
        assert_eq!(live.duration, 0);
        assert_eq!(live.protocol.as_deref(), Some("m3u8_native"));
    }
}
