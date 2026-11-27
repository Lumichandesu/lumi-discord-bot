//! Listening history kept in SQLite (`data/stats.sqlite`), genre detection, and the stats embeds.
//!
//! The schema and queries match the TypeScript bot, so an existing database keeps working.
//! Every database call runs on tokio's blocking pool, never on the async workers.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{Connection, params};
use serde::Deserialize;
use serenity::all::{CreateEmbed, Timestamp};

use crate::util::truncate_chars;

pub const DB_PATH: &str = "data/stats.sqlite";

const SCHEMA: &str = "
  CREATE TABLE IF NOT EXISTS play_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    genre TEXT NOT NULL DEFAULT 'Other',
    duration INTEGER NOT NULL DEFAULT 0,
    played_at DATETIME DEFAULT CURRENT_TIMESTAMP
  );

  CREATE INDEX IF NOT EXISTS idx_guild_played ON play_history(guild_id, played_at);
  CREATE INDEX IF NOT EXISTS idx_guild_user ON play_history(guild_id, user_id);
  CREATE INDEX IF NOT EXISTS idx_guild_genre ON play_history(guild_id, genre);
  CREATE INDEX IF NOT EXISTS idx_guild_artist ON play_history(guild_id, artist);
";

/// The genre cache is emptied once it holds more than this many entries.
const GENRE_CACHE_MAX: usize = 2000;
const ITUNES_TIMEOUT: Duration = Duration::from_secs(2);

/// One track start, as written to `play_history`.
#[derive(Debug, Clone)]
pub struct Play {
    pub guild_id: String,
    pub user_id: String,
    pub title: String,
    /// The uploader as resolved; may be empty.
    pub artist: String,
    pub duration: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenreStat {
    pub genre: String,
    pub count: i64,
    pub percentage: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemStat {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ServerSummary {
    pub total_plays: i64,
    pub unique_tracks: i64,
    pub top_genres: Vec<GenreStat>,
    pub top_artists: Vec<ItemStat>,
    pub top_tracks: Vec<ItemStat>,
    /// `(user_id, plays)`.
    pub top_listeners: Vec<ItemStat>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct UserSummary {
    pub total_plays: i64,
    pub top_genres: Vec<GenreStat>,
    pub top_artists: Vec<ItemStat>,
    pub top_tracks: Vec<ItemStat>,
}

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    // journal_mode returns a row, so it cannot go through execute().
    conn.query_row("PRAGMA journal_mode = WAL;", [], |_| Ok(()))?;
    conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
    conn.execute_batch(SCHEMA)
}

pub fn insert(conn: &Connection, play: &Play, genre: &str) -> rusqlite::Result<()> {
    let artist = if play.artist.is_empty() { "Unknown Artist" } else { &play.artist };
    conn.prepare_cached(
        "INSERT INTO play_history (guild_id, user_id, title, artist, genre, duration) VALUES (?,?,?,?,?,?)",
    )?
    .execute(params![play.guild_id, play.user_id, play.title, artist, genre, play.duration as i64])?;
    Ok(())
}

fn items(conn: &Connection, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<Vec<ItemStat>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(args, |r| Ok(ItemStat { name: r.get(0)?, count: r.get(1)? }))?;
    rows.collect()
}

fn genres(conn: &Connection, sql: &str, args: impl rusqlite::Params, total: i64) -> rusqlite::Result<Vec<GenreStat>> {
    Ok(items(conn, sql, args)?
        .into_iter()
        .map(|i| GenreStat { percentage: round_percent(i.count, total), genre: i.name, count: i.count })
        .collect())
}

/// `Math.round(count / total * 100)` for non-negative values.
fn round_percent(count: i64, total: i64) -> i64 {
    (count as f64 / total as f64 * 100.0 + 0.5).floor() as i64
}

pub fn server_summary(conn: &Connection, guild: &str) -> rusqlite::Result<ServerSummary> {
    let (total_plays, unique_tracks): (i64, i64) = conn.query_row(
        "SELECT COUNT(*) AS total, COUNT(DISTINCT title) AS uniqueTracks FROM play_history WHERE guild_id = ?",
        [guild],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if total_plays == 0 {
        return Ok(ServerSummary::default());
    }
    Ok(ServerSummary {
        total_plays,
        unique_tracks,
        top_genres: genres(
            conn,
            "SELECT genre, COUNT(*) AS count FROM play_history WHERE guild_id = ? GROUP BY genre ORDER BY count DESC LIMIT 5",
            [guild],
            total_plays,
        )?,
        top_artists: items(
            conn,
            "SELECT artist AS name, COUNT(*) AS count FROM play_history WHERE guild_id = ? AND artist != 'Unknown Artist' GROUP BY artist ORDER BY count DESC LIMIT 5",
            [guild],
        )?,
        top_tracks: items(
            conn,
            "SELECT title AS name, COUNT(*) AS count FROM play_history WHERE guild_id = ? GROUP BY title ORDER BY count DESC LIMIT 5",
            [guild],
        )?,
        top_listeners: items(
            conn,
            "SELECT user_id AS userId, COUNT(*) AS count FROM play_history WHERE guild_id = ? GROUP BY user_id ORDER BY count DESC LIMIT 5",
            [guild],
        )?,
    })
}

pub fn user_summary(conn: &Connection, guild: &str, user: &str) -> rusqlite::Result<UserSummary> {
    let total_plays: i64 = conn.query_row(
        "SELECT COUNT(*) AS total FROM play_history WHERE guild_id = ? AND user_id = ?",
        [guild, user],
        |r| r.get(0),
    )?;
    if total_plays == 0 {
        return Ok(UserSummary::default());
    }
    Ok(UserSummary {
        total_plays,
        top_genres: genres(
            conn,
            "SELECT genre, COUNT(*) AS count FROM play_history WHERE guild_id = ? AND user_id = ? GROUP BY genre ORDER BY count DESC LIMIT 5",
            [guild, user],
            total_plays,
        )?,
        top_artists: items(
            conn,
            "SELECT artist AS name, COUNT(*) AS count FROM play_history WHERE guild_id = ? AND user_id = ? AND artist != 'Unknown Artist' GROUP BY artist ORDER BY count DESC LIMIT 5",
            [guild, user],
        )?,
        top_tracks: items(
            conn,
            "SELECT title AS name, COUNT(*) AS count FROM play_history WHERE guild_id = ? AND user_id = ? GROUP BY title ORDER BY count DESC LIMIT 5",
            [guild, user],
        )?,
    })
}

pub fn leaderboard(conn: &Connection, guild: &str, limit: i64) -> rusqlite::Result<Vec<ItemStat>> {
    items(
        conn,
        "SELECT title AS name, COUNT(*) AS count FROM play_history WHERE guild_id = ? GROUP BY title ORDER BY count DESC LIMIT ?",
        params![guild, limit],
    )
}

/// Genre from keywords in the lowercased `artist - title` text. Plain substring tests, so short
/// keywords also match inside words ("eve" in "never"), exactly as the old bot did.
pub fn keyword_genre(key: &str) -> Option<&'static str> {
    const RULES: &[(&[&str], &str)] = &[
        (&["j-pop", "anime", "vocaloid", "yoasobi", "radwimps", "aimer", "lisa", "eve", "kawa"], "J-Pop"),
        (&["k-pop", "bts", "blackpink", "twice", "newjeans", "aespa", "ive"], "K-Pop"),
        (&["lofi", "lo-fi", "chillhop", "relaxing", "study"], "Lo-Fi"),
        (&["hiphop", "hip-hop", "rap", "trap"], "Hip-Hop"),
        (&["rock", "metal", "punk", "alternative"], "Rock"),
        (&["edm", "electronic", "house", "techno", "synth"], "Electronic"),
        (&["jazz", "blues", "soul"], "Jazz"),
    ];
    RULES.iter().find(|(words, _)| words.iter().any(|w| key.contains(w))).map(|(_, g)| *g)
}

/// The iTunes search term: `artist title` cut to 80 UTF-16 units, as JavaScript's `slice` does.
/// `None` when the cut would split a surrogate pair (the old bot's encoder threw there, and the
/// genre became "Other").
pub fn itunes_term(artist: &str, title: &str) -> Option<String> {
    let full = format!("{artist} {title}");
    let mut units = 0;
    let mut out = String::new();
    for c in full.chars() {
        let len = c.len_utf16();
        if units + len > 80 {
            return (units == 80).then_some(out);
        }
        units += len;
        out.push(c);
    }
    Some(out)
}

pub struct Stats {
    conn: Arc<Mutex<Connection>>,
    http: reqwest::Client,
    genre_cache: Mutex<HashMap<String, String>>,
}

impl Stats {
    /// Opens (or creates) the database file, creating its folder if needed.
    pub fn open(path: &Path, http: reqwest::Client) -> Result<Self, String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
        init(&conn).map_err(|e| format!("could not prepare {}: {e}", path.display()))?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)), http, genre_cache: Mutex::new(HashMap::new()) })
    }

    async fn with_conn<R: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<R> + Send + 'static,
    ) -> Result<R, String> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|e| e.into_inner());
            f(&conn)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
    }

    fn cache_genre(&self, key: String, genre: &str) -> String {
        let mut cache = self.genre_cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() > GENRE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, genre.to_string());
        genre.to_string()
    }

    /// Keyword match first, then the public iTunes search API, then "Other". Results are cached.
    pub async fn detect_genre(&self, title: &str, artist: &str) -> String {
        let key = format!("{artist} - {title}").to_lowercase();
        if let Some(g) = self.genre_cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return g.clone();
        }
        if let Some(g) = keyword_genre(&key) {
            return self.cache_genre(key, g);
        }
        let genre = match itunes_term(artist, title) {
            Some(term) => self.itunes_genre(&term).await,
            None => None,
        };
        self.cache_genre(key, genre.as_deref().unwrap_or("Other"))
    }

    async fn itunes_genre(&self, term: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct Search {
            results: Option<Vec<Item>>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Item {
            primary_genre_name: Option<String>,
        }
        let request = async {
            let res = self
                .http
                .get("https://itunes.apple.com/search")
                .query(&[("term", term), ("entity", "song"), ("limit", "1")])
                .send()
                .await
                .ok()?;
            if !res.status().is_success() {
                return None;
            }
            let body: Search = res.json().await.ok()?;
            body.results?.into_iter().next()?.primary_genre_name.filter(|g| !g.is_empty())
        };
        tokio::time::timeout(ITUNES_TIMEOUT, request).await.ok().flatten()
    }

    /// Records a play in the background. Never blocks or fails playback.
    pub fn record(self: &Arc<Self>, play: Play) {
        let stats = self.clone();
        tokio::spawn(async move {
            let genre = stats.detect_genre(&play.title, &play.artist).await;
            if let Err(e) = stats.with_conn(move |c| insert(c, &play, &genre)).await {
                tracing::error!("[Stats Log Error] {e}");
            }
        });
    }

    pub async fn server(&self, guild: String) -> Result<ServerSummary, String> {
        self.with_conn(move |c| server_summary(c, &guild)).await
    }

    pub async fn user(&self, guild: String, user: String) -> Result<UserSummary, String> {
        self.with_conn(move |c| user_summary(c, &guild, &user)).await
    }

    pub async fn top(&self, guild: String) -> Result<Vec<ItemStat>, String> {
        self.with_conn(move |c| leaderboard(c, &guild, 10)).await
    }
}

/// `█` for each filled eighth, `░` for the rest.
pub fn progress_bar(percent: i64) -> String {
    let filled = ((percent as f64 / 100.0 * 8.0 + 0.5).floor() as i64).clamp(0, 8) as usize;
    "█".repeat(filled) + &"░".repeat(8 - filled)
}

fn genre_lines(genres: &[GenreStat], with_count: bool) -> String {
    genres
        .iter()
        .map(|g| {
            let mut line = format!("`{:<12}` `{}` **{}%**", g.genre, progress_bar(g.percentage), g.percentage);
            if with_count {
                line += &format!(" ({})", g.count);
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn artist_lines(artists: &[ItemStat]) -> String {
    artists.iter().enumerate().map(|(i, a)| format!("`{}.` **{}** — `{} plays`", i + 1, a.name, a.count)).collect::<Vec<_>>().join("\n")
}

fn track_lines(tracks: &[ItemStat]) -> String {
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| format!("`{}.` **{}** — `{}x`", i + 1, truncate_chars(&t.name, 30), t.count))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn server_embed(s: &ServerSummary, guild_name: &str) -> CreateEmbed {
    let embed = CreateEmbed::new()
        .color(0x5865f2)
        .title(truncate_chars(&format!("📊 Music Analytics — {guild_name}"), 256))
        .timestamp(Timestamp::now());
    if s.total_plays == 0 {
        return embed.description("No listening history recorded yet. Start playing music to generate stats!");
    }
    let mut embed = embed
        .description(format!("**Total Tracks Played:** `{}`\n**Unique Tracks:** `{}`", s.total_plays, s.unique_tracks));
    if !s.top_genres.is_empty() {
        embed = embed.field("🎧 Top Music Genres", field_value(genre_lines(&s.top_genres, true)), false);
    }
    if !s.top_artists.is_empty() {
        embed = embed.field("🎤 Top Artists", field_value(artist_lines(&s.top_artists)), true);
    }
    if !s.top_tracks.is_empty() {
        embed = embed.field("🎵 Most Played Songs", field_value(track_lines(&s.top_tracks)), true);
    }
    if !s.top_listeners.is_empty() {
        let lines = s
            .top_listeners
            .iter()
            .enumerate()
            .map(|(i, u)| format!("`{}.` <@{}> — `{} plays`", i + 1, u.name, u.count))
            .collect::<Vec<_>>()
            .join("\n");
        embed = embed.field("🏆 Top Listeners", field_value(lines), false);
    }
    embed
}

pub fn user_embed(s: &UserSummary, username: &str) -> CreateEmbed {
    let embed = CreateEmbed::new()
        .color(0x10b981)
        .title(truncate_chars(&format!("🎧 Personal Listening Stats — {username}"), 256))
        .timestamp(Timestamp::now());
    if s.total_plays == 0 {
        return embed.description("You have not requested any music in this server yet!");
    }
    let mut embed = embed.description(format!("**Total Plays Initiated:** `{}`", s.total_plays));
    if !s.top_genres.is_empty() {
        embed = embed.field("🎼 Your Favorite Genres", field_value(genre_lines(&s.top_genres, false)), false);
    }
    if !s.top_artists.is_empty() {
        embed = embed.field("⭐ Favorite Artists", field_value(artist_lines(&s.top_artists)), true);
    }
    if !s.top_tracks.is_empty() {
        embed = embed.field("🔁 Most Repeated Songs", field_value(track_lines(&s.top_tracks)), true);
    }
    embed
}

pub fn leaderboard_embed(tracks: &[ItemStat], guild_name: &str) -> CreateEmbed {
    let embed = CreateEmbed::new()
        .color(0xf59e0b)
        .title(truncate_chars(&format!("🏆 Top 10 Chart — {guild_name}"), 256))
        .timestamp(Timestamp::now());
    if tracks.is_empty() {
        return embed.description("No song data recorded yet!");
    }
    let list = tracks
        .iter()
        .enumerate()
        .map(|(i, t)| format!("`#{}` **{}**\n└ 📈 `{} total plays`", i + 1, t.name, t.count))
        .collect::<Vec<_>>()
        .join("\n\n");
    embed.description(truncate_chars(&list, 4096).to_string())
}

/// Discord rejects embed fields over 1024 characters; long titles or artist names could reach it.
fn field_value(s: String) -> String {
    truncate_chars(&s, 1024).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn
    }

    fn play(conn: &Connection, guild: &str, user: &str, title: &str, artist: &str, genre: &str) {
        let p = Play {
            guild_id: guild.into(),
            user_id: user.into(),
            title: title.into(),
            artist: artist.into(),
            duration: 200,
        };
        insert(conn, &p, genre).unwrap();
    }

    #[test]
    fn keyword_rules_in_order_without_word_boundaries() {
        assert_eq!(keyword_genre("yoasobi - idol"), Some("J-Pop"));
        assert_eq!(keyword_genre("rick astley - never gonna give you up"), Some("J-Pop"), "eve in never");
        assert_eq!(keyword_genre("someone - live at wembley"), Some("K-Pop"), "ive in live");
        assert_eq!(keyword_genre("lofi girl - beats to study to"), Some("Lo-Fi"));
        assert_eq!(keyword_genre("x - trap queen"), Some("Hip-Hop"));
        assert_eq!(keyword_genre("x - punk song"), Some("Rock"));
        assert_eq!(keyword_genre("x - deep house mix"), Some("Electronic"));
        assert_eq!(keyword_genre("x - blue jazz"), Some("Jazz"));
        assert_eq!(keyword_genre("adele - hello"), None);
        // J-Pop is checked before K-Pop.
        assert_eq!(keyword_genre("bts - anime opening"), Some("J-Pop"));
    }

    #[test]
    fn itunes_term_cuts_at_80_utf16_units() {
        assert_eq!(itunes_term("a", "b").as_deref(), Some("a b"));
        let long = "x".repeat(100);
        assert_eq!(itunes_term("a", &long).unwrap().len(), 80);
        // 79 units, then a two-unit emoji that would be split.
        let split = format!("{}😀", "y".repeat(77));
        assert_eq!(itunes_term("a", &split), None);
        let fits = format!("{}😀", "y".repeat(76));
        assert_eq!(itunes_term("a", &fits).map(|s| s.encode_utf16().count()), Some(80));
    }

    #[tokio::test]
    async fn genre_cache_returns_keyword_hits() {
        let stats = Stats {
            conn: Arc::new(Mutex::new(db())),
            http: reqwest::Client::new(),
            genre_cache: Mutex::new(HashMap::new()),
        };
        assert_eq!(stats.detect_genre("Idol", "YOASOBI").await, "J-Pop");
        assert_eq!(stats.genre_cache.lock().unwrap().get("yoasobi - idol").map(String::as_str), Some("J-Pop"));
    }

    #[test]
    fn schema_is_idempotent_and_defaults_played_at() {
        let conn = db();
        init(&conn).unwrap();
        play(&conn, "1", "2", "T", "", "Other");
        let (artist, played_at): (String, String) =
            conn.query_row("SELECT artist, played_at FROM play_history", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(artist, "Unknown Artist");
        assert_eq!(played_at.len(), "YYYY-MM-DD HH:MM:SS".len());
    }

    #[test]
    fn server_summary_aggregates() {
        let conn = db();
        assert_eq!(server_summary(&conn, "g").unwrap(), ServerSummary::default());
        play(&conn, "g", "u1", "Song A", "Artist X", "Rock");
        play(&conn, "g", "u1", "Song A", "Artist X", "Rock");
        play(&conn, "g", "u2", "Song B", "", "Pop");
        play(&conn, "other", "u1", "Song C", "Artist Y", "Jazz");

        let s = server_summary(&conn, "g").unwrap();
        assert_eq!(s.total_plays, 3);
        assert_eq!(s.unique_tracks, 2);
        assert_eq!(
            s.top_genres,
            vec![
                GenreStat { genre: "Rock".into(), count: 2, percentage: 67 },
                GenreStat { genre: "Pop".into(), count: 1, percentage: 33 },
            ]
        );
        // "Unknown Artist" is left out of the artist ranking.
        assert_eq!(s.top_artists, vec![ItemStat { name: "Artist X".into(), count: 2 }]);
        assert_eq!(s.top_tracks[0], ItemStat { name: "Song A".into(), count: 2 });
        assert_eq!(s.top_listeners[0], ItemStat { name: "u1".into(), count: 2 });
        assert_eq!(s.top_listeners.len(), 2);
    }

    #[test]
    fn user_summary_and_leaderboard() {
        let conn = db();
        assert_eq!(user_summary(&conn, "g", "u1").unwrap(), UserSummary::default());
        for i in 0..12 {
            for _ in 0..=i {
                play(&conn, "g", "u1", &format!("Song {i}"), "A", "Rock");
            }
        }
        play(&conn, "g", "u2", "Song 0", "B", "Jazz");
        let u = user_summary(&conn, "g", "u2").unwrap();
        assert_eq!(u.total_plays, 1);
        assert_eq!(u.top_genres, vec![GenreStat { genre: "Jazz".into(), count: 1, percentage: 100 }]);
        assert_eq!(u.top_artists, vec![ItemStat { name: "B".into(), count: 1 }]);

        let top = leaderboard(&conn, "g", 10).unwrap();
        assert_eq!(top.len(), 10);
        assert_eq!(top[0], ItemStat { name: "Song 11".into(), count: 12 });
        assert_eq!(top[9], ItemStat { name: "Song 2".into(), count: 3 });
        assert!(leaderboard(&conn, "none", 10).unwrap().is_empty());
    }

    #[test]
    fn progress_bar_rounds_to_eighths() {
        assert_eq!(progress_bar(0), "░░░░░░░░");
        assert_eq!(progress_bar(100), "████████");
        assert_eq!(progress_bar(50), "████░░░░");
        assert_eq!(progress_bar(6), "░░░░░░░░");
        assert_eq!(progress_bar(7), "█░░░░░░░");
        assert_eq!(round_percent(1, 3), 33);
        assert_eq!(round_percent(1, 2), 50);
        assert_eq!(round_percent(1, 8), 13);
    }
}
