import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { shuffleQueue, requireGuild, requireVoiceChannel } from "../../music/player";

export const shuffleCommand: BotCommand = {
  name: "shuffle",
  description: "Shuffle queue",
  async execute(message: Message) {
    const guild = requireGuild(message);
    requireVoiceChannel(message);
    const count = shuffleQueue(guild.id);
    await message.reply(`🔀 Shuffled **${count}** tracks.`);
  },
};
