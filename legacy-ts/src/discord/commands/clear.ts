import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { clearQueue, requireGuild, requireVoiceChannel } from "../../music/player";

export const clearCommand: BotCommand = {
  name: "clear",
  description: "Clear queue",
  async execute(message: Message) {
    const guild = requireGuild(message);
    requireVoiceChannel(message);
    const count = clearQueue(guild.id);
    await message.reply(`🗑️ Cleared **${count}** track(s).`);
  },
};
