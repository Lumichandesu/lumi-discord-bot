import {
  EmbedBuilder,
  ActionRowBuilder,
  ButtonBuilder,
  ButtonStyle,
} from "discord.js";

export interface TrackInfoDisplay {
  title: string;
  uri: string;
  author: string;
  duration: number;
  thumbnail: string | null;
}

export function formatDuration(seconds: number): string {
  if (!seconds || isNaN(seconds)) return "Live / Unknown";
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s < 10 ? "0" : ""}${s}`;
}

export function createNowPlayingEmbed(
  track: TrackInfoDisplay,
  isPaused = false,
  repeatMode: "off" | "track" | "queue" = "off"
) {
  const embed = new EmbedBuilder()
    .setColor(isPaused ? 0xf59e0b : 0x5865f2)
    .setAuthor({ name: isPaused ? "⏸️ Playback Paused" : "🎵 Now Playing" })
    .setTitle(track.title.length > 60 ? `${track.title.slice(0, 57)}...` : track.title)
    .setURL(track.uri)
    .addFields(
      { name: "Artist", value: `\`${track.author || "Unknown"}\``, inline: true },
      { name: "Duration", value: `\`${formatDuration(track.duration)}\``, inline: true },
      { name: "Loop", value: `\`${repeatMode.toUpperCase()}\``, inline: true }
    )
    .setTimestamp();

  if (track.thumbnail) {
    embed.setThumbnail(track.thumbnail);
  }

  const row1 = new ActionRowBuilder<ButtonBuilder>().addComponents(
    new ButtonBuilder()
      .setCustomId("btn_pause")
      .setEmoji(isPaused ? "▶️" : "⏸️")
      .setStyle(isPaused ? ButtonStyle.Success : ButtonStyle.Secondary),
    new ButtonBuilder()
      .setCustomId("btn_skip")
      .setEmoji("⏭️")
      .setStyle(ButtonStyle.Secondary),
    new ButtonBuilder()
      .setCustomId("btn_loop")
      .setEmoji("🔁")
      .setStyle(repeatMode !== "off" ? ButtonStyle.Primary : ButtonStyle.Secondary),
    new ButtonBuilder()
      .setCustomId("btn_shuffle")
      .setEmoji("🔀")
      .setStyle(ButtonStyle.Secondary),
    new ButtonBuilder()
      .setCustomId("btn_stop")
      .setEmoji("⏹️")
      .setStyle(ButtonStyle.Danger)
  );

  const row2 = new ActionRowBuilder<ButtonBuilder>().addComponents(
    new ButtonBuilder()
      .setCustomId("btn_queue")
      .setLabel("Queue")
      .setEmoji("📜")
      .setStyle(ButtonStyle.Secondary),
    new ButtonBuilder()
      .setCustomId("btn_lyrics")
      .setLabel("Lyrics")
      .setEmoji("📝")
      .setStyle(ButtonStyle.Secondary)
  );

  return {
    embeds: [embed],
    components: [row1, row2],
  };
}
