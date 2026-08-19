import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { getPlayer, requireGuild } from "../../music/player";

export const nowPlayingCommand: BotCommand = {
  name: "nowplaying",
  description: "Display current track",
  async execute(message: Message) {
    const guild = requireGuild(message);
    const player = getPlayer(guild.id);
    const current = player?.queue.current;
    if (!player || !current) {
      await message.reply("🔇 Nothing is playing.");
      return;
    }
    await message.reply(`🎵 **Now Playing:** [${current.info.title}](${current.info.uri})\n👤 **Artist:** ${current.info.author || "Unknown"}\n🔁 **Loop Mode:** \`${player.repeatMode}\``);
  },
};
