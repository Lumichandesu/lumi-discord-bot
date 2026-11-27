import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { skip, requireGuild, requireVoiceChannel } from "../../music/player";

export const skipCommand: BotCommand = {
  name: "skip",
  description: "Skip current track",
  async execute(message: Message) {
    const guild = requireGuild(message);
    requireVoiceChannel(message);
    const skipped = await skip(guild.id);
    await message.reply(`⏭️ Skipped: **${skipped?.info.title ?? "Current track"}**`);
  },
};
