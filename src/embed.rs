//! The now-playing embed and its control buttons.

use serenity::all::{
    ButtonStyle, CreateActionRow, CreateButton, CreateEmbed, CreateEmbedAuthor, CreateEmbedFooter, ReactionType, Timestamp,
};

use crate::queue::RepeatMode;
use crate::resolver::Track;
use crate::util::{format_duration, truncate_chars};

pub const COLOR_MAIN: u32 = 0x5865f2;
const COLOR_PAUSED: u32 = 0xf59e0b;

fn button(id: &str, emoji: &str, style: ButtonStyle) -> CreateButton {
    CreateButton::new(id).emoji(ReactionType::Unicode(emoji.into())).style(style)
}

/// `platform` is only known when a track is announced; command replies show `NATIVE AUDIO`.
pub fn now_playing(
    track: &Track,
    paused: bool,
    repeat: RepeatMode,
    platform: Option<&str>,
) -> (CreateEmbed, Vec<CreateActionRow>) {
    let title = if track.title.chars().count() > 60 {
        format!("{}...", truncate_chars(&track.title, 57))
    } else {
        track.title.clone()
    };
    let author = if track.author.is_empty() { "Unknown" } else { &track.author };
    let mut embed = CreateEmbed::new()
        .color(if paused { COLOR_PAUSED } else { COLOR_MAIN })
        .author(CreateEmbedAuthor::new(if paused { "⏸️ Playback Paused" } else { "🎵 Now Playing" }))
        .title(title)
        .url(&track.url)
        .field("Artist", format!("`{author}`"), true)
        .field("Duration", format!("`{}`", format_duration(track.duration)), true)
        .field("Loop", format!("`{}`", repeat.as_str().to_uppercase()), true)
        .field("Bitrate", "`128 kbps (HD)`", true)
        .field("Codec", "`Opus 48kHz`", true)
        .field("Platform", format!("`{}`", platform.unwrap_or("Native Audio").to_uppercase()), true)
        .timestamp(Timestamp::now())
        .footer(CreateEmbedFooter::new("Lumi Bot | Ultra-Low Latency Audio"));
    if let Some(thumb) = &track.thumbnail {
        embed = embed.thumbnail(thumb);
    }

    let looping = repeat != RepeatMode::Off;
    let row1 = CreateActionRow::Buttons(vec![
        button("btn_pause", if paused { "▶️" } else { "⏸️" }, if paused { ButtonStyle::Success } else { ButtonStyle::Secondary }),
        button("btn_skip", "⏭️", ButtonStyle::Secondary),
        button("btn_loop", "🔁", if looping { ButtonStyle::Primary } else { ButtonStyle::Secondary }),
        button("btn_shuffle", "🔀", ButtonStyle::Secondary),
        button("btn_stop", "⏹️", ButtonStyle::Danger),
    ]);
    let row2 = CreateActionRow::Buttons(vec![
        button("btn_queue", "📜", ButtonStyle::Secondary).label("Queue"),
        button("btn_lyrics", "📝", ButtonStyle::Secondary).label("Lyrics"),
    ]);
    (embed, vec![row1, row2])
}
