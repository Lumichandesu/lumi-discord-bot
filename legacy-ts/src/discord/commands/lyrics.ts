import { EmbedBuilder, type Message } from "discord.js";
import type { BotCommand } from "./types";
import { getPlayer } from "../../music/player";

export const lyricsCommand: BotCommand = {
  name: "lyrics",
  description: "Get lyrics for the current song or search query",
  async execute(message: Message, args: string[]) {
    let query = args.join(" ").trim();
    const guild = message.guild;

    if (!query && guild) {
      const player = getPlayer(guild.id);
      if (player?.queue.current) {
        query = `${player.queue.current.info.title} ${player.queue.current.info.author}`;
      }
    }

    if (!query) {
      await message.reply("📝 Usage: `\\lyrics <song title>` (or play a song first).");
      return;
    }

    const searching = await message.reply("🔍 Searching lyrics...");

    try {
      const res = await fetch(
        `https://lrclib.net/api/search?q=${encodeURIComponent(query)}`,
        { signal: AbortSignal.timeout(5000) }
      );

      if (!res.ok) throw new Error("Lyrics service unavailable");
      const results: any = await res.json();

      if (!Array.isArray(results) || results.length === 0 || !results[0]?.plainLyrics) {
        await searching.edit(`❌ No lyrics found for **${query}**.`);
        return;
      }

      const item = results[0];
      const lyrics = item.plainLyrics.slice(0, 3900);

      const embed = new EmbedBuilder()
        .setColor(0x5865f2)
        .setTitle(`📝 ${item.trackName} - ${item.artistName}`)
        .setDescription(lyrics)
        .setFooter({ text: "Lyrics provided by LRCLIB" })
        .setTimestamp();

      await searching.edit({ content: null, embeds: [embed] });
    } catch (err) {
      await searching.edit("❌ Failed to retrieve lyrics.");
    }
  },
};
