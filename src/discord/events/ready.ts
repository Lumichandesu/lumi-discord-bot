import type { Client } from "discord.js";
import { logger } from "../../utils/logger";

export function registerReadyEvent(
  client: Client,
) {
  client.once("clientReady", (readyClient) => {
    logger.info(
      `Logged in as ${readyClient.user.tag}`,
    );

    logger.info(
      `Bot ID: ${readyClient.user.id}`,
    );

    logger.info(
      `Guild count: ${readyClient.guilds.cache.size}`,
    );

    logger.info(
      "Lumi Bot is ready.",
    );
  });
}
