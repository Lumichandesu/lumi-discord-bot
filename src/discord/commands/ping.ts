import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { getLavalink } from "../../music/lavalink";

export const pingCommand: BotCommand = {
  name: "ping",
  description: "Check bot latency and Lavalink node ping",
  async execute(message: Message) {
    const sent = await message.reply("🏓 Pinging...");

    const roundtrip = sent.createdTimestamp - message.createdTimestamp;
    const wsPing = message.client.ws.ping;

    let nodePingText = "N/A";
    try {
      const lavalink = getLavalink();
      const node = lavalink.nodeManager.leastUsedNodes()[0];
      if (node && node.connected) {
        const start = Date.now();
        await node.fetchInfo();
        nodePingText = `${Date.now() - start}ms`;
      }
    } catch {
      // Node offline or request failed
    }

    const text = [
      "🏓 **Pong!**",
      `📡 **Message Latency:** \`${roundtrip}ms\``,
      `💓 **Discord WebSocket:** \`${wsPing >= 0 ? `${wsPing}ms` : "Connecting..."}\``,
      `🎵 **Lavalink Node:** \`${nodePingText}\``,
    ].join("\n");

    await sent.edit(text);
  },
};
