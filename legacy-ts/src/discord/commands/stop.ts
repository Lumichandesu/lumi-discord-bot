import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { stop, requireGuild, requireVoiceChannel } from "../../music/player";

export const stopCommand: BotCommand = {
  name: "stop",
  description: "Stop playback and clear queue",
  async execute(message: Message) {
    const guild = requireGuild(message);
    requireVoiceChannel(message);
    await stop(guild.id);
    await message.reply("⏹️ Stopped and cleared queue.");
  },
};
