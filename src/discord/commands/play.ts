import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { play } from "../../music/player";
import { createNowPlayingEmbed } from "../components/playerEmbed";

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

      if (result.queued) {
        await message.reply(`🎵 Added to queue: **${result.track.info.title}**`);
      } else {
        const ui = createNowPlayingEmbed(result.track.info, false, "off");
        await message.reply(ui);
      }
    } catch (error) {
      console.error("[Play]", error);
      await message.reply(
        `❌ ${error instanceof Error ? error.message : "Unable to play this track."}`,
      );
    }
  },
};
