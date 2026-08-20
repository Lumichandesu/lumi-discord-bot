import { EmbedBuilder, type Message } from "discord.js";
import type { BotCommand } from "./types";

export const pingCommand: BotCommand = {
  name: "ping",
  description: "Check bot performance and system metrics",
  async execute(message: Message) {
    const start = Date.now();
    const sent = await message.reply("Pinging...");
    const latency = Date.now() - start;
    const ws = message.client.ws.ping;
    
    // System Metrics
    const uptime = process.uptime();
    const mem = process.memoryUsage().heapUsed / 1024 / 1024;
    
    const formatUptime = (seconds: number) => {
      const h = Math.floor(seconds / 3600);
      const m = Math.floor((seconds % 3600) / 60);
      const s = Math.floor(seconds % 60);
      return `${h}h ${m}m ${s}s`;
    };

    const embed = new EmbedBuilder()
      .setTitle("🏓 Pong!")
      .setColor(0x0099ff)
      .addFields(
        { 
          name: "📡 Network Latency", 
          value: `• WebSocket: \`${ws}ms\`\n• Roundtrip: \`${latency}ms\``, 
          inline: true 
        },
        { 
          name: "🖥️ System Metrics", 
          value: `• Uptime: \`${formatUptime(uptime)}\`\n• Heap RAM: \`${mem.toFixed(2)} MB\``, 
          inline: true 
        }
      )
      .setTimestamp()
      .setFooter({ text: "Lumi Bot | Optimized for e2-micro" });

    await sent.edit({ content: null, embeds: [embed] });
  },
};
