//! Lyrics lookup through the public LRCLIB API (https://lrclib.net).

use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    pub track_name: Option<String>,
    pub artist_name: Option<String>,
    pub plain_lyrics: Option<String>,
}

/// Returns the first search result, or `Ok(None)` when LRCLIB has no plain lyrics for the query.
pub async fn search(http: &reqwest::Client, query: &str, timeout: Duration) -> Result<Option<Lyrics>, String> {
    let res = http
        .get("https://lrclib.net/api/search")
        .query(&[("q", query)])
        .timeout(timeout)
        .send()
        .await
        .map_err(|_| "Lyrics service unavailable".to_string())?
        .error_for_status()
        .map_err(|_| "Lyrics service unavailable".to_string())?;
    let results: Vec<Lyrics> = res.json().await.map_err(|_| "Lyrics service unavailable".to_string())?;
    Ok(results.into_iter().next().filter(|l| l.plain_lyrics.as_deref().is_some_and(|t| !t.trim().is_empty())))
}
