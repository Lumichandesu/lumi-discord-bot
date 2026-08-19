import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { removeQueueTrack, requireGuild, requireVoiceChannel } from "../../music/player";

export const removeCommand: BotCommand = {
  name: "remove",
  description: "Remove track by index",
  async execute(message: Message, args: string[]) {
    const guild = requireGuild(message);
    requireVoiceChannel(message);
    const index = parseInt(args[0] ?? "", 10);
    if (isNaN(index)) {
      await message.reply("❌ Usage: `\\remove <number>`");
      return;
    }
    const removed = removeQueueTrack(guild.id, index);
    await message.reply(`🗑️ Removed \`#${index}\`: **${removed.info.title}**`);
  },
};
