import { Client, GatewayIntentBits } from "discord.js";
import { env } from "./config/env";
import { startApiServer } from "./server";
import { registerMessageHandler } from "./discord/events/messages";

const client = new Client({
  intents: [
    GatewayIntentBits.Guilds,
    GatewayIntentBits.GuildVoiceStates,
    GatewayIntentBits.GuildMessages,
    GatewayIntentBits.MessageContent,
  ],
});

registerMessageHandler(client);

client.once("ready", () => {
  console.log(`[INFO] Logged in as ${client.user?.tag}`);
  console.log(`[INFO] Bot ID: ${client.user?.id}`);
  console.log(`[INFO] Guild count: ${client.guilds.cache.size}`);

  startApiServer(client, 3000);
  console.log("[INFO] Lumi Bot is ready.");
});

client.login(env.DISCORD_TOKEN);
