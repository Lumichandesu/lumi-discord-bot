import type { Message } from "discord.js";

export interface BotCommand {
  name: string;
  description: string;
  execute(
    message: Message,
    args: string[],
  ): Promise<void>;
}
