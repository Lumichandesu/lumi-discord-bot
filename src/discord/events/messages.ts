import type {
  Client,
  Message,
} from "discord.js";

import {
  commandMap,
} from "../commands";

import {
  logger,
} from "../../utils/logger";

const PREFIX = "\\";

export function registerMessageHandler(
  client: Client,
): void {
  client.on(
    "messageCreate",
    async (message: Message) => {
      if (message.author.bot) {
        return;
      }

      if (!message.content.startsWith(PREFIX)) {
        return;
      }

      const content =
        message.content.slice(
          PREFIX.length,
        ).trim();

      if (!content) {
        return;
      }

      const parts =
        content.split(/\s+/);

      const commandName =
        parts.shift()?.toLowerCase();

      if (!commandName) {
        return;
      }

      const command =
        commandMap.get(commandName);

      if (!command) {
        return;
      }

      try {
        await command.execute(
          message,
          parts,
        );
      } catch (error) {
        logger.error(
          `Command failed: \\${commandName}`,
          error,
        );

        await message.reply(
          "❌ An internal error occurred.",
        );
      }
    },
  );
}
