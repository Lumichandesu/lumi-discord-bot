import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { pause, requireGuild, requireVoiceChannel } from "../../music/player";

export const pauseCommand: BotCommand = {
  name: "pause",
  description: "Pause playback",
  async execute(message: Message) {
    const guild = requireGuild(message);
    requireVoiceChannel(message);
    await pause(guild.id);
    await message.reply("⏸️ Playback paused.");
  },
};
