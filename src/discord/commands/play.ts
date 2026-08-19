import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { play } from "../../music/player";

export const playCommand: BotCommand = {
  name: "play",
  description: "Play a YouTube, SoundCloud, Spotify, Apple Music, or TIDAL URL / Search query",
  async execute(message: Message, args: string[]) {
    const query = args.join(" ").trim();

    if (!query) {
      await message.reply("🎵 Usage: `\\play <Song Title / URL (YouTube, Spotify, Apple Music, TIDAL, SoundCloud)>`");
      return;
    }

    try {
      const result = await play(message, query);

      await message.reply(
        result.queued
          ? `🎵 Added to queue: **${result.track.info.title}**`
          : `▶️ Playing: **${result.track.info.title}**`,
      );
    } catch (error) {
      console.error("[Play]", error);
      await message.reply(
        `❌ ${error instanceof Error ? error.message : "Unable to play this track."}`,
      );
    }
  },
};
