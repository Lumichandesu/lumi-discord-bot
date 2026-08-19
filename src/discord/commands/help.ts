import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { commands } from "./index";

export const helpCommand: BotCommand = {
  name: "help",
  description: "Display all available commands",
  async execute(message: Message) {
    const list = commands
      .map((cmd) => `• \`\\${cmd.name}\` - ${cmd.description}`)
      .join("\n");

    const text = [
      "📖 **Lumi Bot - Command List**",
      "",
      list,
      "",
      "💡 *Aliases:* `\\q` (queue), `\\np` (nowplaying), `\\next` (skip), `\\leave` (stop), `\\repeat` (loop)",
    ].join("\n");

    await message.reply(text);
  },
};
