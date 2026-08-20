import { EmbedBuilder, type Message } from "discord.js";
import type { BotCommand } from "./types";

export const helpCommand: BotCommand = {
  name: "help",
  description: "Display all available commands and usage guide",
  async execute(message: Message) {
    const embed = new EmbedBuilder()
      .setColor(0x5865f2)
      .setTitle("📖 Lumi Bot — Commands Guide")
      .setDescription("Prefix: `\\` | Interactive buttons are also available below player embeds.")
      .addFields(
        {
          name: "🎵 Music Playback",
          value: [
            "`\\play <title/url>` — Play audio from YouTube, Spotify, Apple Music, TIDAL, SoundCloud",
            "`\\pause` — Pause current playback",
            "`\\resume` — Resume playback",
            "`\\skip` — Skip to the next track",
            "`\\stop` — Stop music and disconnect bot",
            "`\\seek <mm:ss>` — Jump to timestamp (e.g. `\\seek 1:45`)",
          ].join("\n"),
        },
        {
          name: "📜 Queue & Controls",
          value: [
            "`\\queue` — Show current track and upcoming list",
            "`\\nowplaying` — Show detailed info of the current song",
            "`\\loop` — Cycle loop mode (`OFF` ➔ `TRACK` ➔ `QUEUE`)",
            "`\\shuffle` — Shuffle upcoming tracks",
            "`\\remove <#>` — Remove a specific track from queue",
            "`\\clear` — Clear entire queue",
          ].join("\n"),
        },
        {
          name: "🛠️ Utilities",
          value: [
            "`\\lyrics [query]` — Fetch synchronized/plain lyrics",
            "`\\ping` — Check latency and system performance",
            "`\\help` — Show this command list",
          ].join("\n"),
        }
      )
      .setFooter({ text: "Lumi Bot | Ultra-Low RAM & Anti-Lag Edition" })
      .setTimestamp();

    await message.reply({ embeds: [embed] });
  },
};
