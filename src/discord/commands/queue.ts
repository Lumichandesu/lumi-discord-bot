import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { getPlayer, requireGuild } from "../../music/player";

export const queueCommand: BotCommand = {
  name: "queue",
  description: "Show music queue",
  async execute(message: Message) {
    const guild = requireGuild(message);
    const player = getPlayer(guild.id);
    if (!player || (!player.queue.current && player.queue.tracks.length === 0)) {
      await message.reply("📭 Queue is empty.");
      return;
    }
    const current = player.queue.current;
    let text = `🎶 **Now Playing:** ${current ? `[${current.info.title}](${current.info.uri})` : "None"}\n\n`;
    if (player.queue.tracks.length > 0) {
      text += `📜 **Queue (${player.queue.tracks.length} tracks):**\n`;
      player.queue.tracks.slice(0, 10).forEach((t, i) => {
        text += `\`${i + 1}.\` [${t.info.title}](${t.info.uri})\n`;
      });
      if (player.queue.tracks.length > 10) {
        text += `\n*...and ${player.queue.tracks.length - 10} more*`;
      }
    }
    await message.reply(text);
  },
};
