import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { resume, requireGuild, requireVoiceChannel } from "../../music/player";

export const resumeCommand: BotCommand = {
  name: "resume",
  description: "Resume playback",
  async execute(message: Message) {
    const guild = requireGuild(message);
    requireVoiceChannel(message);
    await resume(guild.id);
    await message.reply("▶️ Playback resumed.");
  },
};
