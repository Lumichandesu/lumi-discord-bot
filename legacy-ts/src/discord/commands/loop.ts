import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { toggleLoop, requireGuild, requireVoiceChannel } from "../../music/player";

export const loopCommand: BotCommand = {
  name: "loop",
  description: "Toggle loop mode",
  async execute(message: Message) {
    const guild = requireGuild(message);
    requireVoiceChannel(message);
    const mode = toggleLoop(guild.id);
    await message.reply(`🔁 Loop mode set to: \`${mode}\``);
  },
};
